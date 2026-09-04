//! The render loop's source may not contain what the render loop may not do.
//!
//! The counting-allocator test covers allocation behaviourally. Three of the four rules
//! it cannot see: a lock, an I/O call, and a logging macro all leave the allocator alone
//! while breaking the audio thread just as thoroughly. This is a **lint-grade** check
//! over a named region of files, each of which exists as its own file so the region needs
//! no exceptions — `prepare` allocates legitimately, and a scan over one file holding
//! both could only be a scan with holes.
//!
//! # The region, and why it grew
//!
//! Phase 1's region was one file: `src/render/hot.rs`. Phase 2 dispatches through a
//! prepared function table, so the loop reaches code that is not in it — and
//! [ADR-0004](../../../plans/v2/decisions/ADR-0004-native-node-representation.md) clause
//! 4 makes that a hard requirement rather than a preference: **every callee reachable
//! from the loop must be enumerable from source**, because the phase's real-time
//! guarantee is a source-level transitive check. A dispatch shape whose callee set could
//! not be enumerated was excluded by that clause regardless of its speed.
//!
//! So the region is the loop plus the kernels, the callee set is the registry, and
//! [`the_kernel_registry_is_closed_and_no_scanned_form_forges_a_kernel`] checks that the
//! registry and the kernels agree, as far as reading the source can establish.
//! Phase 3 adds `src/schedule/hot.rs`, which selects a borrowed compiled-event span
//! before calling the renderer and is subject to the same rules.
//!
//! What this cannot do is stop someone moving code out of the region to dodge it. That is
//! true of any structural check; what it does stop is the ordinary case, where a helper
//! that locks or logs is added to the hot path without anyone noticing.

use std::path::{Path, PathBuf};

/// The files the real-time rules cover, relative to the crate root.
///
/// The three `hot.rs` files are the arbiter, scheduler and renderer loops. `kernels.rs`
/// is everything the renderer calls through the prepared function table — and the registry
/// that resolves those pointers is deliberately *not* here, because it runs at
/// admission, allocates, and reads the IR.
///
/// `src/publish/hot.rs` joined when the publication arbiter did. It runs on the audio
/// thread ahead of the renderer, so leaving it out would have left the one path that
/// *writes* renderer input unscanned while the path that reads it was covered.
///
/// `src/ingress/hot.rs` joined with the live ingress store, for the same reason and one
/// more: it is the only file in the region that **writes back** into a producer's own
/// storage while the call runs, so an allocation there would be one the producing half
/// never sees.
const REGION: [&str; 7] = [
    "src/render/hot.rs",
    "src/render/slot.rs",
    "src/schedule/hot.rs",
    "src/publish/hot.rs",
    "src/identity/hot.rs",
    "src/ingress/hot.rs",
    "src/node/kernels.rs",
];

fn read_region_file(relative: &str) -> String {
    let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Every file in the region, as (name, source).
fn region_sources() -> Vec<(&'static str, String)> {
    REGION
        .iter()
        .map(|relative| (*relative, read_region_file(relative)))
        .collect()
}

fn hot_path_source() -> String {
    read_region_file("src/render/hot.rs")
}

fn scheduler_hot_path_source() -> String {
    read_region_file("src/schedule/hot.rs")
}

fn publication_hot_path_source() -> String {
    read_region_file("src/publish/hot.rs")
}

/// The file's code lines, with comments and doc comments removed.
///
/// A comment naming a forbidden construct is how the reasons get recorded — this file's
/// own header does it — so scanning them would make the check unusable.
fn code_lines(source: &str) -> Vec<(usize, String)> {
    source
        .lines()
        .enumerate()
        .map(|(index, line)| (index + 1, line.trim().to_owned()))
        .filter(|(_, line)| {
            !line.is_empty()
                && !line.starts_with("//")
                && !line.starts_with("///")
                && !line.starts_with("//!")
        })
        .collect()
}

#[test]
fn the_render_loop_takes_no_lock_and_performs_no_io_or_logging() {
    let forbidden = [
        ("Mutex", "a blocking lock"),
        ("RwLock", "a blocking lock"),
        (".lock(", "a blocking lock"),
        (".read(", "a blocking lock or an I/O read"),
        (".write(", "a blocking lock or an I/O write"),
        ("println!", "logging"),
        ("eprintln!", "logging"),
        ("dbg!", "logging"),
        ("tracing::", "logging"),
        ("log::", "logging"),
        ("std::fs", "filesystem I/O"),
        ("File::", "filesystem I/O"),
        ("std::thread", "scheduling"),
        ("std::time", "a clock read"),
    ];

    let mut found = Vec::new();
    for (file, source) in region_sources() {
        for (line_number, line) in code_lines(&source) {
            for (needle, why) in forbidden {
                if line.contains(needle) {
                    found.push(format!(
                        "{file}:{line_number} contains `{needle}` ({why}): {line}"
                    ));
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "the render loop must not do these things:\n  {}",
        found.join("\n  ")
    );
}

#[test]
fn the_render_loop_contains_no_allocating_construct() {
    let forbidden = [
        "Vec::new",
        "Vec::with_capacity",
        "vec![",
        "to_vec()",
        "String::",
        "format!(",
        "to_owned()",
        "to_string()",
        "Box::new",
        "HashMap",
        "BTreeMap",
        "collect()",
        ".sort_by_key(",
        ".sort_by(",
        ".sort(",
        ".push(",
        ".insert(",
        ".resize(",
        ".extend(",
    ];

    let mut found = Vec::new();
    for (file, source) in region_sources() {
        for (line_number, line) in code_lines(&source) {
            for needle in forbidden {
                if line.contains(needle) {
                    found.push(format!("{file}:{line_number} contains `{needle}`: {line}"));
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "the render loop must not allocate or grow a collection:\n  {}",
        found.join("\n  ")
    );
}

#[test]
fn the_render_loop_cannot_panic_through_an_unchecked_accessor() {
    // `unwrap` and `expect` are forbidden in production code repository-wide; on the
    // audio thread a panic is worse than a wrong sample. The stable sort is banned above
    // for a different reason — `sort_by_key` allocates while `sort_unstable_by_key` does
    // not — and this is where the panicking accessors are caught.
    let forbidden = [
        "unwrap()",
        "unwrap_or_else(",
        "expect(",
        "panic!",
        "unreachable!",
        "assert",
    ];

    let mut found = Vec::new();
    for (file, source) in region_sources() {
        for (line_number, line) in code_lines(&source) {
            for needle in forbidden {
                if line.contains(needle) {
                    found.push(format!("{file}:{line_number} contains `{needle}`: {line}"));
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "the render loop must not be able to panic:\n  {}",
        found.join("\n  ")
    );
}

#[test]
fn the_check_is_reading_the_file_it_thinks_it_is() {
    // A scan over an empty or renamed file passes every assertion above. This is the
    // control: the file has to be the render loop.
    let source = hot_path_source();
    assert!(source.len() > 2_000, "hot.rs is unexpectedly small");
    for expected in [
        "impl Renderer for PreparedRenderer",
        "fn render_quantum",
        "fn resolve_events",
        "sort_unstable_by_key",
    ] {
        assert!(
            source.contains(expected),
            "hot.rs does not contain `{expected}`; the render loop moved and this check is now \
             scanning the wrong thing"
        );
    }

    let publication = publication_hot_path_source();
    assert!(
        publication.len() > 1_000,
        "publish/hot.rs is unexpectedly small"
    );
    // The arbiter is the one path that *writes* renderer input, so a scan that quietly
    // stopped covering it would leave the write side unguarded while the read side stayed
    // green. Each name is load-bearing: `charge` is where the share is enforced, `seal`
    // is what makes the batch immutable, and `row_of` is what derives the destination from
    // the event rather than taking it from the caller.
    for expected in [
        "impl<'a> Publication<'a>",
        "pub fn charge",
        "pub fn seal",
        "fn row_of",
    ] {
        assert!(
            publication.contains(expected),
            "publish/hot.rs does not contain `{expected}`; the publication path moved and this \
             check is now scanning the wrong thing"
        );
    }

    let identity = read_region_file("src/identity/hot.rs");
    assert!(
        identity.len() > 500,
        "identity/hot.rs is unexpectedly small"
    );
    // Resolving and releasing run on the audio thread when a note edge is applied; minting
    // stays in `table.rs`, off it. The split is what lets this file be scanned at all — the
    // region is file-granular and `table.rs` allocates in its constructor.
    for expected in ["pub fn resolve", "pub fn release", "pub fn note_of"] {
        assert!(
            identity.contains(expected),
            "identity/hot.rs does not contain `{expected}`; the resolving path moved and this \
             check is now scanning the wrong thing"
        );
    }

    let slot = read_region_file("src/render/slot.rs");
    assert!(slot.len() > 1_000, "render/slot.rs is unexpectedly small");
    // The slot is where a law is applied and where a write is composed; if either moved,
    // the composition scan below would be reading a file that composes nothing.
    for expected in [
        "impl SlotState",
        "fn write_override",
        "fn resolve(self, base: ParameterValue",
    ] {
        assert!(
            slot.contains(expected),
            "render/slot.rs does not contain `{expected}`; the parameter slot moved and this              check is now scanning the wrong thing"
        );
    }

    let scheduler = scheduler_hot_path_source();
    assert!(
        scheduler.len() > 1_000,
        "schedule/hot.rs is unexpectedly small"
    );
    for expected in [
        "impl CompiledEventScheduler",
        "pub fn render",
        "quanta_needed_for",
    ] {
        assert!(
            scheduler.contains(expected),
            "schedule/hot.rs does not contain `{expected}`; the scheduler hot path moved and \
             this check is now scanning the wrong thing"
        );
    }
}

#[test]
fn the_render_loop_makes_no_topology_or_naming_decision() {
    // The Phase 2 gate adds four constructs to the three Phase 1 checked: no port
    // strings, no map lookups, no graph traversal or topology decision, no buffer
    // resizing. `HashMap` and `.resize(` are already banned by the allocation check
    // above, so these are the rest — the identities the compiler is supposed to have
    // turned into slots, and the searches it is supposed to have made unnecessary.
    let forbidden = [
        (
            "PortId",
            "a port identity is a naming decision the compiler owed",
        ),
        ("PortName", "a port string"),
        (
            "NodeId",
            "an identity that should have been compiled to a slot",
        ),
        (
            "ParameterId",
            "an identity that should have been compiled to a slot",
        ),
        ("GraphIr", "graph traversal"),
        ("IrNodeKind", "a topology decision"),
        ("IrEdge", "graph traversal"),
        (
            ".find(",
            "a search where the compiler could have produced an index",
        ),
        (
            ".iter().position(",
            "a search where the compiler could have produced an index",
        ),
    ];

    let mut found = Vec::new();
    for (file, source) in region_sources() {
        for (line_number, line) in code_lines(&source) {
            for (needle, why) in forbidden {
                if line.contains(needle) {
                    found.push(format!(
                        "{file}:{line_number} contains `{needle}` ({why}): {line}"
                    ));
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "the render loop must decide nothing about topology or naming:\n  {}",
        found.join("\n  ")
    );
}

#[test]
fn every_call_the_render_loop_makes_is_inside_the_checked_region() {
    // What the scans above cannot do on their own: someone moves a locking helper into
    // another file and calls it from here, and every check still passes. The file's own
    // header said exactly that. This closes it.
    //
    // Every call in this file — method, associated function, or free function, on any
    // receiver, however the line breaks fall — must match either something defined in
    // this file or a name on the allowlist below. The allowlist is not a convenience:
    // each group is justified, and a name that is not on it fails this test until
    // someone justifies it here.
    //
    // **What this cannot do, stated rather than implied.** It matches *names*, not
    // resolved targets: a helper elsewhere called `get` on some other receiver would
    // pass. Closing that needs type resolution, which a source scan does not have. Two
    // things bound the gap. The import check below means a free function cannot be
    // brought in at all, so an escape has to be a method on a type this file already
    // touches. And the counting-allocator test covers the allocation half behaviourally
    // whatever the call is named. What is left is a locking or blocking method on an
    // existing receiver, deliberately named like an accessor — and the allowlist is
    // short enough that a reader checking it would notice.
    let region: Vec<(&str, String)> = region_sources()
        .into_iter()
        .map(|(file, source)| (file, strip_comments(&source)))
        .collect();
    let source: String = region
        .iter()
        .map(|(_, text)| text.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    let defined: Vec<String> = source
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            trimmed
                .strip_prefix("fn ")
                .or_else(|| trimmed.strip_prefix("pub fn "))
                .or_else(|| trimmed.strip_prefix("pub const fn "))
                .or_else(|| trimmed.strip_prefix("const fn "))
                .or_else(|| trimmed.strip_prefix("pub(crate) fn "))
                .or_else(|| trimmed.strip_prefix("pub(crate) const fn "))
        })
        .filter_map(|rest| rest.split(['(', '<']).next())
        .map(|name| name.trim().to_owned())
        .collect();

    // Control flow, visibility and attributes, none of which are calls. `pub(crate)`,
    // `#[derive(..)]` and `#[cfg(..)]` are each an identifier followed by a parenthesis and
    // are neither reachable code nor nameable as a function. `cfg` is here because the slot
    // module compiles its modulation seam for tests only, until Phase 7 gives it a caller.
    let syntax = [
        "if", "while", "for", "match", "return", "loop", "else", "fn", "let", "move", "pub",
        "derive", "cfg",
    ];

    // `std` on slices, `Option`, iterators and primitives: bounds-checked accessors,
    // fills, a memmove, saturating integer arithmetic, and the one in-place sort. None
    // allocates, locks, or can panic — `get`/`get_mut` return `Option` precisely so the
    // loop indexes without panicking, and `sort_unstable_by_key` is the non-allocating
    // sort where `sort_by_key` would allocate.
    let std_calls = [
        "get",
        "get_mut",
        // `Result::is_ok` and `Result::is_err`: a discriminant read on a value the caller
        // owns. Clause 4's split path uses them to sequence its two sub-calls and to decide
        // whether the callback must be silenced whole, without moving the outcome it
        // returns. Neither allocates, locks or can panic.
        "is_ok",
        "is_err",
        // `core::mem::take` and `core::mem::swap`: moves between values the caller already
        // owns. ADR-0050 clause 3's adoption exchanges the candidate's vectors with the
        // scheduler's live ones rather than building a second value, which is what keeps the
        // audio thread from allocating a retired container. Neither call touches the
        // allocator, and the counting-allocator test covers that behaviourally.
        // `Option::take`: a move out plus a `None` write, on storage the caller already
        // holds. It is how the live-note registry's scoped mass release ends a note without
        // reading the slot twice, and it neither allocates nor can panic.
        "take",
        "fill",
        "len",
        "is_empty",
        "copied",
        "copy_within",
        "copy_from_slice",
        "iter",
        "enumerate",
        "contains",
        "sort_unstable_by_key",
        "unwrap_or",
        "map_or",
        "ok",
        "is_some_and",
        // The live boundary's counters and the report they reach. `diagnostics_mut` hands
        // back a `&mut` to a field of the renderer; `mirror_ingress_boundary` writes four
        // `u64`s into it; and `dropped_slot`, `dropped_hold`, `dropped_identity`,
        // `orphan_releases` and `beyond_horizon` read five `u64`s out of a `Copy` struct.
        // `HOST-INV-009` requires the counts to reach the structured report, and the drain is
        // the only point at which the producing and rendering halves meet — so this crossing
        // is contractual rather than convenient. `beyond_horizon` joined when `HOST-INV-013`'s
        // single evaluation moved to the ingress boundary, which is the same crossing for the
        // same reason. None of the seven allocates, locks or can panic, and the
        // counting-allocator test covers the drain behaviourally.
        "diagnostics_mut",
        "mirror_ingress_boundary",
        "dropped_slot",
        "dropped_hold",
        "dropped_identity",
        "orphan_releases",
        "beyond_horizon",
        // `Option::is_some` and `Option::as_deref_mut`: a discriminant read, and a
        // reborrow of a `&mut` the caller already holds. The second is what lets one
        // optional ingress store be drained by **both** halves of a split render call —
        // moving the reference instead would serve only the first half, which is a
        // correctness bug rather than a real-time one, but neither call reaches the
        // allocator either way.
        "is_some",
        "as_deref_mut",
        // `Option::as_deref`: the shared-reference twin, used to read the ingress store's
        // identity for the latch before the store is moved into the drain. A discriminant
        // read and a reborrow; it reaches no allocator.
        "as_deref",
        // `PerformanceIngress::adopted_by`: an `Option<StreamEpoch>` copied out of the
        // store. It is what lets the drain read the **one** adoption mark the off-thread
        // half set, instead of keeping a second latch that could disagree with it. A field
        // read; it reaches no allocator.
        "adopted_by",
        "try_from",
        "from",
        "ok_or",
        "map_err",
        "default",
        "checked_sub",
        "saturating_mul",
        "saturating_add",
        "saturating_sub",
        "div_ceil",
        "min",
        "max",
        "abs",
        // The parameter slot's two exponential laws (`SOUND-INV-023`): `f32::exp2` for the
        // semitone law and `f32::powf` for the decibel law. Both are pure libm calls on a
        // value the slot owns — no allocation, no lock, and no panic: an overflow is an
        // infinity, which `ParameterValue::saturating` then holds to the finite domain.
        // They run per **write**, not per frame; `SOUND-INV-024`'s per-frame advance is an
        // add, and lands in the same file under the same scan.
        "exp2",
        "powf",
        // `f32::clamp`, in the slot's two additive laws and the level's domain hold. It can
        // panic only on an inverted or `NaN` range, so the composition scan below holds
        // every `clamp` in the region to two literal bounds — the same argument the
        // `saturating` entry makes for the one clamp that lives outside the region.
        "clamp",
        // `ModulationLaw::identity`: a `const fn` match over the law enum returning the sum an
        // unmodulated slot holds. Read once, when a slot is prepared, and it neither
        // allocates, locks nor panics.
        "identity",
        // `NoteVelocity::saturating` is the documented policy holding a parameter-written
        // velocity inside `[0, 1]`, which is the destination's domain — `SOUND-INV-021` puts
        // the *refusing* constructor on the note payload, and the parameter path has only
        // this one. It is a `NaN` test and an `f32::clamp`: two comparisons and a select,
        // allocating nothing and locking nothing. `clamp` can panic on `min > max` alone,
        // and both are literals there, so the assertion is constant-true and compiles away;
        // `NaN` is answered before it is reached. It lives on the quantity rather than at
        // the assignment because `AGENTS.md` requires a documented saturating **type** to
        // own a policy like this, which is what moved it out of the checked region and onto
        // this list.
        "saturating",
        // `SOUND-INV-021`'s expansion, both halves of it. `CompiledPlan::note_magnitudes_of`
        // is a plan-identity comparison and one checked slice of a table the plan owns;
        // `CompiledPlan::magnitude_value` is two array indexes and an infallible conversion,
        // which is the whole reason a key resolves through a **prepared** table rather than
        // through a formula. Neither allocates, locks, or panics: every lookup is `get`, and
        // an out-of-range slot yields the empty slice or `None` rather than a substituted
        // frequency. They live on the plan rather than in the loop because the plan is what
        // owns the tables, and a copy of them in `hot.rs` would be a second authority on
        // where a note's magnitudes land.
        "note_magnitudes_of",
        "magnitude_value",
        "floor",
        "sin",
        // `f64::is_finite` is one exponent comparison and compiles to a bit test. The
        // sawtooth's band-limiting residual uses it to refuse a step it cannot place in its
        // domain, which is the alternative to a negated partial-order comparison that reads
        // as a trap. It allocates nothing, locks nothing, and cannot panic.
        "is_finite",
        "matches",
        // The arbiter's half. `measured` is `EventCount`'s observation constructor — a
        // `const fn` wrapping a `u32`, which is exactly the newtype the critical rule asks
        // for in place of a bare count, and it allocates nothing. It appears in the hot
        // path because a fault and an occupancy both carry their unit rather than a raw
        // integer.
        "measured",
        // `ProducerId::as_u16` is a `const fn` field read, used to index a producer's range
        // when a scoped mass release picks its span. It allocates nothing and locks nothing.
        "as_u16",
        // The kernels' half: in-place iteration over preallocated slices, the borrow
        // split that turns slots into slices, and one sort over a fixed-size array of
        // three entries. `split_at_mut_checked` returns `Option` rather than panicking,
        // and `sort_unstable` is the non-allocating sort.
        "iter_mut",
        "zip",
        "flatten",
        "first",
        "checked_mul",
        "split_at_mut_checked",
        "sort_unstable",
        "size_of",
        // `Option::map` over a `Copy` payload, in the three helpers that turn an event
        // payload into the node, control and value a sample-positioned change moves.
        "map",
        // `Option::and_then` in the two helpers that resolve a note edge's node and control.
        // A note-off carries only an occurrence, so resolving it is itself fallible, and the
        // slot lookup that follows is fallible too — chaining is what keeps both bounds
        // checks rather than indexing past one. The closure is a slice `get`.
        "and_then",
    ];

    // This crate's own `const fn` accessors and `Copy` constructors: field reads on the
    // prepared plan, the anchor, the clock, the event envelope, and the diagnostics
    // counters, plus `new` on the checked time and count newtypes. Each is a field read
    // or a saturating add.
    let crate_accessors = [
        "new",
        // ADR-0050's adoption path, all `const fn` field reads or saturating counters.
        //
        // `carry_frames` is how clause 4 finds where a crossing host block is cut, and
        // `split_at_frame` is the cut itself — one `split_at_mut` over the caller's own
        // buffer, describing the same samples as two spans rather than copying them.
        // `requested` is the candidate's immutable stamp. `count_late_activation` is a
        // saturating add on the diagnostics report, exactly like the counters already here.
        "carry_frames",
        "split_at_frame",
        "requested",
        "count_late_activation",
        // `count_displacement_fault` is the same saturating add on the diagnostics report,
        // on the terminal path an activation displacement takes. It exists apart from the
        // publication counter so the two terminal conditions stay attributable.
        "count_displacement_fault",
        // `reborrow` and `silence` are the other half of clause 4's split. A crossing block
        // is handed to two renderer calls, and the terminal contract is silence over the
        // **complete** callback rather than over the span a fault happened in — so the split
        // path has to keep both halves after lending them. `reborrow` is a shorter borrow of
        // a slice the caller already holds; `silence` is one `fill(0.0)` over it. Neither
        // allocates, locks or can panic, and the block was shape-checked at construction.
        "reborrow",
        "silence",
        // `charge_operation` is ADR-0046 clause 6's bounded mass release charged as **one**
        // unit of the session share. It is defined in `publish/hot.rs`, inside the scanned
        // region, and does the same index-checked ledger arithmetic `charge` does without
        // writing an event into the batch.
        "charge_operation",
        // `release_all` is the registry's scoped mass release: a bounded walk over one
        // producer's span, clearing slots and reporting their nodes into storage the
        // candidate carries. It grows nothing and its bound is checked before it writes.
        "release_all",
        "note_targets",
        "channel_layout",
        "forward_event_horizon",
        "max_events_per_quantum",
        "maximum_block_size",
        "ops",
        // ADR-0041 clause 2: the plan **records** where each slot's samples live, so the
        // loop reads a region's offset and length where it used to multiply a slot index
        // by the quantum. Clause 4 adds the other two: a step carries the layout of the
        // signal it writes, and the binding hands the kernel its channel count. All are
        // field reads on `Copy` types.
        "regions",
        "region",
        "offset",
        "length",
        "out_layout",
        "channels",
        "parameter_targets",
        "sample_rate",
        "id",
        // `NoteIdentity::table` is a `const fn` field read. The renderer compares it against
        // its own table's id to reject a foreign occurrence, the same shape the foreign-slot
        // check already had — a comparison, not a lookup, so a stale identity from another
        // plan never indexes anything.
        "table",
        // The live-note registry's three: `admit` writes one preallocated slot, `note_of`
        // reads one, and `release` clears one. All three are indexed writes into storage
        // preparation sized to the whole admitted partition, so none can grow and none can
        // fail for want of room.
        "admit",
        "note_of",
        "release",
        "plan",
        "index",
        "position",
        "time",
        // Phase 3's compiled scheduler reads the renderer's current quantum boundary
        // before selecting the immutable event slice for the call.
        "clock",
        "epoch",
        "envelope",
        "payload",
        "source",
        "is_ingress",
        "is_negative",
        "difference",
        "checked_add",
        "checked_advance_quantum",
        "quantum_index",
        "as_u64",
        "as_i64",
        "as_f32",
        "as_usize",
        "as_slice",
        "into_frequency",
        "into_amplitude",
        "frames",
        "layout",
        "needs_reprepare",
        "set_needs_reprepare",
        "count_late_event",
        "count_stale_epoch_event",
        "count_out_of_horizon_event",
        "count_arrival_stamped_event",
        "count_foreign_slot_event",
        "count_orphan_note_event",
        "count_oversized_callback",
        "count_publication_fault",
        // `PreparedRenderer::diagnostics` and `DiagnosticsReport::needs_reprepare` are
        // `const fn` field reads. The scheduler consults them before publishing so a dead
        // epoch is not faulted a second time, which is a read of state the loop already
        // owns rather than new work.
        "diagnostics",
        "count_clock_exhaustion",
        // Phase 2's additions: the compiled step's slots, the prepared table, and the
        // one method that moves a control. Each is a field read or an assignment.
        "prepared_nodes",
        "node",
        "out",
        "inputs",
        "in_place_safe",
        "kernel",
        "set_control",
        "bind",
        // The segment counter's own accessors: a field read, a comparison with zero, and
        // a saturating decrement.
        "is_finished",
        "spent",
        // The binding a step resolved at admission, read rather than recomputed.
        "order",
        "bindings",
        // P02-T007's additions, all field reads on `Copy` types: the plan's note address
        // table, a position's offset inside its quantum — a remainder — and the control
        // value a note edge names, which is a match over two variants returning a
        // constant.
        "note_targets",
        "quantum_offset",
        "value",
    ];

    // The one module-qualified call in the region, and it is **not on the hot path at
    // all**.
    //
    // `Kernel::is_same` backs `NodeStep::same_kernel`, which compares two *schedules* —
    // a compile-time question a test asks, never something a quantum does. It lives in
    // `kernels.rs` because `Kernel`'s pointer is private to that module: the comparison
    // had to move in when the pointer stopped being nameable from outside it.
    //
    // Justified on its own terms as well as by not being reached: it compares two
    // function addresses. It cannot allocate, lock, panic, or reach a device.
    let pointer_comparison = [
        "std::ptr::fn_addr_eq",
        // ADR-0050's adoption exchanges the candidate's vectors with the scheduler's live
        // ones rather than building a second value, which is what keeps the audio thread
        // from allocating a container for the retired state. Both are moves between values
        // the caller already owns: no allocator call, no lock, no panic. They are justified
        // by their full paths rather than by the bare-name list, so `take` and `swap` on
        // some other receiver would still have to answer for themselves.
        "core::mem::take",
        "core::mem::swap",
    ];

    let mut unresolved: Vec<String> = Vec::new();
    let bytes: Vec<char> = source.chars().collect();
    for (index, character) in bytes.iter().enumerate() {
        if *character != '(' {
            continue;
        }
        // The identifier immediately before the parenthesis, if any.
        let mut start = index;
        while start > 0
            && bytes
                .get(start - 1)
                .is_some_and(|c| c.is_alphanumeric() || *c == '_')
        {
            start -= 1;
        }
        if start == index {
            continue;
        }
        let name: String = bytes[start..index].iter().collect();
        if name.chars().next().is_some_and(char::is_uppercase) {
            // A tuple-struct or enum-variant construction, not a call.
            continue;
        }

        // A **module-qualified** call reaches out of this file whatever it is named, and
        // matching only the final identifier would accept `crate::elsewhere::fault(..)`
        // through a local `fn fault`. The qualifier decides: `Type::new(..)` is a
        // constructor on a `Copy` newtype, `module::helper(..)` is code this check
        // cannot see.
        let qualifier: String = bytes[..start]
            .iter()
            .rev()
            .take_while(|c| c.is_alphanumeric() || **c == '_' || **c == ':')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        // Rust's primitives are types whose names are lowercase, so they are the one
        // qualifier that looks like a module and is not one. `u32::try_from` and
        // `f64::from` are conversions on `Copy` values.
        let primitives = [
            "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize",
            "f32", "f64", "bool", "char", "str",
        ];
        if qualifier.ends_with("::") {
            let owner = qualifier.trim_end_matches("::").rsplit("::").next();
            let is_module = owner.is_some_and(|segment| {
                !primitives.contains(&segment)
                    && segment
                        .chars()
                        .next()
                        .is_some_and(|first| first.is_lowercase() || first == '_')
            });
            // A qualifier that names a **region module** is not a reach outside the
            // checked region: `kernels::bind` is code this scan reads, and refusing it
            // would mean the loop could not call a kernel at all. Anything else
            // module-qualified is exactly what the scan cannot see.
            let region_module = owner.is_some_and(|segment| REGION_MODULES.contains(&segment));
            let qualified = format!("{qualifier}{name}");
            // Justified by its full path, so it is settled here rather than falling
            // through to the bare-name lists — which would admit the same identifier on
            // any receiver.
            if pointer_comparison.contains(&qualified.as_str()) {
                continue;
            }
            if is_module && !region_module && !unresolved.contains(&qualified) {
                unresolved.push(qualified);
                continue;
            }
        }
        if syntax.contains(&name.as_str())
            || std_calls.contains(&name.as_str())
            || crate_accessors.contains(&name.as_str())
            || defined.contains(&name)
        {
            continue;
        }
        if !unresolved.contains(&name) {
            unresolved.push(name);
        }
    }

    assert!(
        unresolved.is_empty(),
        "the render loop calls names that are neither defined in hot.rs nor justified in \
         this test's allowlist, so the real-time guarantee does not cover them: {unresolved:?}"
    );

    // A control: the scan has to be finding calls at all.
    assert!(
        defined.iter().any(|name| name == "render_quantum"),
        "the definition scan found no `render_quantum`, so it is not reading this file"
    );
}

#[test]
fn the_render_loop_imports_no_free_function() {
    // The half of reachability a name scan *can* settle. Every `use` in the hot path
    // must name a type, a trait, or a constant — all of which start with an uppercase
    // letter — so a free function cannot be brought into scope to be called. That is the
    // loophole the check above would otherwise leave wide: a lowercase import is the one
    // way to call code elsewhere without a receiver this file already holds.
    // Whole items, not lines: `render/hot.rs` already has a multiline brace import, and a
    // line-based scan would skip every name inside it — which is precisely where a
    // lowercase helper would be added.
    let mut imported_functions = Vec::new();
    let mut seen_by_file = Vec::new();

    for (file, source) in region_sources() {
        let source = strip_comments(&source).replace('\n', " ");
        let mut seen = 0;
        for item in source.split(';') {
            let trimmed = item.trim();
            let Some(rest) = trimmed.strip_prefix("use ") else {
                continue;
            };
            seen += 1;
            let names = rest.replace(['{', '}'], " ");
            for name in names.split([',', ' ']) {
                let Some(last) = name.rsplit("::").next() else {
                    continue;
                };
                let last = last.trim();
                if last.is_empty() || last == "self" || last == "super" || last == "crate" {
                    continue;
                }
                // A glob is worse than a lowercase name: it brings in whatever the other
                // module has, including free functions whose names collide with the
                // accessors the call scan allows. A **region module** is the exception,
                // and the only one: importing `kernels` is how the loop reaches the
                // function table, and everything in it is scanned by this file.
                if last == "*"
                    || (last.chars().next().is_some_and(char::is_lowercase)
                        && !REGION_MODULES.contains(&last))
                {
                    imported_functions.push(format!("{file}: {last}"));
                }
            }
        }
        seen_by_file.push((file, seen));
    }

    assert!(
        imported_functions.is_empty(),
        "the render loop imports lowercase names, which are functions or modules rather \
         than types: {imported_functions:?}"
    );

    // The control: a scan that found nothing at all would also pass the assertion above.
    // Each file has its own floor, so the renderer alone cannot satisfy the control for
    // a scheduler or kernel file that was accidentally removed from `REGION`.
    for (required, floor) in [
        ("src/render/hot.rs", 6),
        ("src/render/slot.rs", 2),
        ("src/schedule/hot.rs", 2),
        ("src/publish/hot.rs", 2),
        ("src/identity/hot.rs", 2),
        ("src/node/kernels.rs", 3),
    ] {
        let seen = seen_by_file
            .iter()
            .find(|(file, _)| *file == required)
            .map(|(_, seen)| *seen)
            .unwrap_or(0);
        assert!(
            seen >= floor,
            "the import scan found {seen} `use` items in {required}, below its control floor \
             {floor}"
        );
    }
}

/// The modules of the region, as they are named in a `use` or a call qualifier.
///
/// A call into one of these is not a reach outside the checked region: this file reads
/// them. Adding a name here without adding its file to [`REGION`] would be the whole
/// check going quietly hollow, and
/// [`the_region_modules_are_all_scanned`] is what stops that.
const REGION_MODULES: [&str; 1] = ["kernels"];

#[test]
fn the_region_modules_are_all_scanned() {
    // The two lists have to agree, or the call scan would wave through a module nobody
    // reads. `kernels` is `src/node/kernels.rs`; the mapping is by file name.
    for module in REGION_MODULES {
        assert!(
            REGION
                .iter()
                .any(|file| file.ends_with(&format!("/{module}.rs"))),
            "`{module}` is trusted by the call and import scans but is not in the scanned region"
        );
    }
}

#[test]
fn a_kernel_composes_nothing_and_the_law_is_applied_in_one_place() {
    // `SOUND-INV-023`'s last clause, by scan. A kernel reads one resolved value: the kernel
    // file names no law, calls no composition, and carries neither exponential the two
    // exponential laws are made of. And the law's arithmetic is defined once, in the slot
    // module, so two native kinds cannot compose one law differently — neither composes,
    // and there is one `resolve` to disagree with.
    let kernels = strip_comments(&read_region_file("src/node/kernels.rs"));
    for forbidden in [
        "ModulationLaw",
        "SlotState",
        ".resolve(",
        "write_override",
        ".exp2(",
        ".powf(",
        "hold_to_domain",
    ] {
        assert!(
            !kernels.contains(forbidden),
            "node/kernels.rs contains `{forbidden}`: a kernel is composing, which              `SOUND-INV-023` forbids"
        );
    }

    // The control: the slot file does contain every one of those, so the scan above is
    // looking for names that exist.
    let slot = strip_comments(&read_region_file("src/render/slot.rs"));
    for expected in [
        "ModulationLaw",
        "SlotState",
        "write_override",
        ".exp2(",
        ".powf(",
        "hold_to_domain",
    ] {
        assert!(
            slot.contains(expected),
            "render/slot.rs does not contain `{expected}`, so the kernel scan is looking for              a name nothing uses"
        );
    }

    // Every `clamp` in the region has two literal bounds, which is the condition under
    // which `f32::clamp` cannot panic and the reason the call scan carries it. A bound that
    // is a name — a value the caller computed — is what this refuses.
    let literal_clamp = regex_free_literal_clamp;
    for (file, source) in region_sources() {
        for (line_number, line) in code_lines(&source) {
            let mut rest = line.as_str();
            while let Some(at) = rest.find(".clamp(") {
                let args = &rest[at + ".clamp(".len()..];
                assert!(
                    literal_clamp(args),
                    "{file}:{line_number}: `{}` clamps to a bound that is not a literal, which is \
                     the one way `f32::clamp` can panic on the audio thread",
                    line.trim()
                );
                rest = args;
            }
        }
    }

    // One definition of the law's arithmetic in the whole crate, and it is the slot's.
    let mut definitions = Vec::new();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut pending = vec![root];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).expect("the source tree is readable") {
            let path = entry.expect("a readable entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let text = std::fs::read_to_string(&path).expect("a readable file");
                if strip_comments(&text).contains("fn resolve(self, base: ParameterValue") {
                    definitions.push(path);
                }
            }
        }
    }
    assert_eq!(
        definitions.len(),
        1,
        "the law's arithmetic is defined in {definitions:?}; `SOUND-INV-023` puts it in one          place"
    );
    assert!(
        definitions[0].ends_with("render/slot.rs"),
        "the law's arithmetic is defined in {:?} rather than the slot",
        definitions[0]
    );
}

/// Whether `args` — the text after `.clamp(` — opens with two numeric literals and a
/// closing parenthesis: `0.0, 1.0)`, `-1.0, 1.0)`, or with an `f32` suffix.
fn regex_free_literal_clamp(args: &str) -> bool {
    let Some((inside, _)) = args.split_once(')') else {
        return false;
    };
    let mut bounds = inside.split(',');
    let (Some(low), Some(high), None) = (bounds.next(), bounds.next(), bounds.next()) else {
        return false;
    };
    let is_literal = |text: &str| {
        let text = text.trim().trim_end_matches("_f32").trim_end_matches("f32");
        !text.is_empty()
            && text
                .strip_prefix('-')
                .unwrap_or(text)
                .chars()
                .all(|c| c.is_ascii_digit() || c == '.' || c == '_')
    };
    is_literal(low) && is_literal(high)
}

/// A `Kernel` cannot be built outside `node::kernels`, and this checks what follows from
/// that but is not implied by it.
///
/// # What changed, and why this test is smaller than it was
///
/// `SOUND-INV-013` says every kernel reachable from the render loop lives in this crate.
/// An earlier form of this check scanned `node.rs` for `kernel: kernels::…` and compared
/// the names it found against the functions defined in `kernels.rs`. That caught a
/// registered name that was not a kernel — but it was keyed on the path it expected, so a
/// descriptor written against *any other* path was invisible to it rather than caught.
/// The specification recorded that as an open gap.
///
/// [`Kernel`](synth_engine_v2::node::kernels::Kernel) is now a newtype whose field is
/// private to `kernels.rs`, so a `Kernel` can only be **constructed there**. A descriptor
/// anywhere else naming any function **does not compile** — verified by mutation: it
/// fails with `E0423, cannot initialize a tuple struct which contains private fields`.
///
/// **That is all rustc carries.** An in-module `Kernel(foreign)` is perfectly well typed,
/// so privacy does not by itself say the declared constants are the only kernels that
/// exist. This test goes after that by reading the source: it rejects the construction
/// spellings it recognises, and checks the registry entries and constants it can parse
/// agree in both directions. Both are scans, and the specification's *Unresolved
/// questions* records what a scan for a grammar cannot be.
#[test]
fn the_kernel_registry_is_closed_and_no_scanned_form_forges_a_kernel() {
    let kernels = strip_comments(&read_region_file("src/node/kernels.rs"));
    let registry = strip_comments(&read_region_file("src/node.rs"));

    // --- What the type system leaves open: a `Kernel` built here over a foreign fn. ---
    //
    // Privacy makes rustc refuse a descriptor naming a function outside this module, and
    // that is the whole of the compiler's contribution. It is **not** the guarantee that
    // only the nine constants exist: code *inside* `kernels.rs` can construct a `Kernel`
    // over any function it can name, and export it.
    //
    // So this scans the file's construction sites, and each must be a declared constant.
    // The field is private, so there are none anywhere else — but the scan itself reads
    // *source forms*, and successive review passes each found another valid spelling it
    // did not recognise: `-> Option<Self>`, an associated `const … : Self`, functional
    // record syntax `Self { 0: … }`, a type alias. Nine spellings are mutation-checked
    // and the brace forms and aliases are covered, and it is still a scan for a grammar
    // rather than a proof. The specification records that boundary rather than implying
    // it away.
    // Construction is `Kernel(…)` or `Kernel { … }` anywhere in the file, and the same
    // two spellings of `Self` inside an `impl Kernel` block — elsewhere in this file
    // `Self` is some other type, and the kernel file has several. `Self { 0: … }` is
    // valid for a tuple struct and is why the brace forms are matched too.
    let constructs = |line: &str| {
        line.contains("Kernel(") || line.contains("Kernel {") && !line.contains("impl Kernel {")
    };
    let mut construction_sites: Vec<&str> = kernels
        .lines()
        .map(str::trim)
        .filter(|line| constructs(line))
        .collect();
    for block in kernels.split("impl Kernel {").skip(1) {
        let body = block.split("\n}").next().unwrap_or("");
        construction_sites.extend(body.lines().map(str::trim).filter(|line| {
            (line.contains("Self(") || line.contains("Self {")) && !constructs(line)
        }));
    }
    assert!(
        construction_sites.len() > 1,
        "found {} construction sites, so this scan is not reading the kernel file",
        construction_sites.len()
    );
    for line in &construction_sites {
        let is_declaration = *line == "pub struct Kernel(KernelFn);";
        let is_constant = line.starts_with("pub const ") && line.contains(": Kernel = Kernel(");
        assert!(
            is_declaration || is_constant,
            "`{line}` constructs a `Kernel` somewhere other than a declared constant. \
             Privacy stops a *descriptor* naming a foreign function, but code in this \
             module can still wrap one and export it, so every construction site has to \
             be one of the constants"
        );
    }
    // An alias would let a construction wear a name this scan does not look for.
    assert!(
        !kernels.contains("Kernel as "),
        "`Kernel` is aliased in the kernel file, which puts a construction beyond the \
         reach of the scan above"
    );

    // --- The functions, the constants and the entries this scan can parse agree. ---
    let defined: Vec<String> = kernels
        .split("pub fn ")
        .skip(1)
        .filter_map(|rest| {
            let name = rest.split(['(', '<']).next()?.trim().to_owned();
            // A kernel is a **free function** with the kernel signature. Two things are
            // excluded and both are in this file: `bind`, which the loop calls directly
            // and which has a different signature, and `Kernel::run`, which takes `self`
            // and *invokes* a kernel rather than being one. ADR-0004 clause 5 is what
            // makes the `self` test decisive — a kernel never takes one.
            //
            // Matched with whitespace removed. `cargo fmt --check` is in the gate, so a
            // differently spaced signature would be caught there too — but a check that
            // silently stops recognising a kernel because someone wrote one space fewer
            // is a check that fails open, and this one must not.
            let parameters: String = rest
                .split_once('(')?
                .1
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            let takes_self = parameters.starts_with("self")
                || parameters.starts_with("&self")
                || parameters.starts_with("&mutself");
            (parameters.contains("io:&mutNodeIo") && !takes_self).then_some(name)
        })
        .collect();
    assert!(
        defined.len() > 3,
        "found {} kernels, so this scan is not reading the kernel file",
        defined.len()
    );

    // `pub const NAME: Kernel = Kernel(function);`
    let constants: Vec<(String, String)> = kernels
        .split("pub const ")
        .skip(1)
        .filter_map(|rest| {
            let (head, tail) = rest.split_once(": Kernel = Kernel(")?;
            Some((
                head.trim().to_owned(),
                tail.split(')').next()?.trim().to_owned(),
            ))
        })
        .collect();
    assert_eq!(
        constants.len(),
        defined.len(),
        "{} kernel functions but {} registrable constants; every kernel needs exactly one",
        defined.len(),
        constants.len()
    );
    for (name, wraps) in &constants {
        assert!(
            defined.contains(wraps),
            "`{name}` wraps `{wraps}`, which is not a function with the kernel signature"
        );
    }
    for function in &defined {
        assert!(
            constants.iter().any(|(_, wraps)| wraps == function),
            "`{function}` has the kernel signature but no `Kernel` constant, so no \
             descriptor can reach it"
        );
    }

    // Every descriptor entry names one of those constants, and every constant is used.
    let entries: Vec<String> = registry
        .split("kernel: kernels::")
        .skip(1)
        .filter_map(|fragment| Some(fragment.split([',', ' ', '\n']).next()?.trim().to_owned()))
        .collect();
    assert!(
        !entries.is_empty(),
        "the registry scan found no `kernel: kernels::` entry, so it is not reading node.rs"
    );
    // No descriptor may spell its kernel any other way. The compiler already refuses a
    // *foreign* function; this refuses a local alias that would hide which constant is
    // meant from a reader of the registry.
    // Minus the field declarations, `kernel: Kernel`, which are not entries — and minus a
    // declaration-derived descriptor's `kernel: self.kernel`, which names no kernel at all:
    // it forwards the one its `NodeDeclaration` already named through `kernels::`, and
    // that naming is counted above. `P05-S001` introduced the form, and holding it to
    // exactly that spelling is what keeps a derivation from becoming a second path.
    let forwarded = registry.matches("kernel: self.kernel").count();
    assert!(
        forwarded <= 1,
        "a kernel is forwarded from a declaration in {forwarded} places; one derivation \
         is the design, more is a second registry"
    );
    assert_eq!(
        registry.matches("kernel: ").count()
            - registry.matches("kernel: Kernel").count()
            - forwarded,
        entries.len(),
        "a descriptor names its kernel by some path other than `kernels::`, so the \
         registry no longer reads as one list of named kernels"
    );
    for entry in &entries {
        assert!(
            constants.iter().any(|(name, _)| name == entry),
            "the registry names `kernels::{entry}`, which is not a `Kernel` constant"
        );
    }
    for (name, _) in &constants {
        assert!(
            entries.contains(name),
            "`{name}` is a registrable kernel that no descriptor uses, so it is dead code"
        );
    }
}

/// The source with comments and string literals removed.
///
/// Both would otherwise feed the scan: this file's own header names several forbidden
/// constructs, and a diagnostic's message text contains parentheses.
fn strip_comments(source: &str) -> String {
    source
        .lines()
        .map(|line| {
            let code = line.split("//").next().unwrap_or("");
            let mut out = String::with_capacity(code.len());
            let mut in_string = false;
            let mut escaped = false;
            for character in code.chars() {
                match character {
                    '"' if !escaped => in_string = !in_string,
                    '\\' if in_string => escaped = !escaped,
                    _ => escaped = false,
                }
                if !in_string && character != '"' {
                    out.push(character);
                }
            }
            out
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `SOUND-INV-012`: adding a node adds no renderer control flow.
///
/// The region already names no `IrNodeKind` — `the_render_loop_makes_no_topology_or_naming_decision`
/// forbids it — so the loop cannot branch on a kind. What that leaves is a second way in:
/// a kind called differently, either through a second dispatch site or by a kernel named
/// directly from the loop. Both are source forms, and both are held here: the hot path
/// dispatches through exactly one `Kernel::run` site, and names no kernel but the
/// kind-independent seam `kernels::bind`.
#[test]
fn the_render_loop_dispatches_every_node_through_one_site() {
    let lines = code_lines(&hot_path_source());
    let dispatch_sites: Vec<&(usize, String)> = lines
        .iter()
        .filter(|(_, line)| line.contains(".kernel().run("))
        .collect();
    assert_eq!(
        dispatch_sites.len(),
        1,
        "the render loop must reach every kernel through one dispatch site, found: {dispatch_sites:?}"
    );

    // Any other `.run(` is a kernel value invoked outside prepared dispatch —
    // `kernels::SAW.run(..)` reaches a kind by name while leaving the count above at one,
    // which an independent review pointed out — so every `.run(` must be that one site.
    let other_runs: Vec<String> = lines
        .iter()
        .filter(|(_, line)| line.contains(".run(") && !line.contains(".kernel().run("))
        .map(|(line_number, line)| format!("{line_number}: {line}"))
        .collect();
    assert!(
        other_runs.is_empty(),
        "the render loop must invoke no kernel value but the dispatched one:\n  {}",
        other_runs.join("\n  ")
    );

    // And no kernel is named at all: a `kernels::` path may reach the kind-independent
    // seam `bind` or a type, never a function call or a `Kernel` constant. A constant is
    // spelled in capitals, a type in camel case, so the spelling decides.
    let named_kernels: Vec<String> = lines
        .iter()
        .filter_map(|(line_number, line)| {
            let start = line.find("kernels::")?;
            let rest = &line[start + "kernels::".len()..];
            let identifier: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            let called = rest[identifier.len()..].starts_with('(');
            let constant = !identifier.is_empty()
                && identifier
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
            ((called && identifier != "bind") || constant).then(|| format!("{line_number}: {line}"))
        })
        .collect();
    assert!(
        named_kernels.is_empty(),
        "the render loop must name no kernel; a kind reached by name is control flow the \
         registry did not add:\n  {}",
        named_kernels.join("\n  ")
    );
}
