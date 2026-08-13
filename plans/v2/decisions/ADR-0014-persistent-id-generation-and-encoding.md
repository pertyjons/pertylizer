# ADR-0014: Persistent ID Generation and Encoding

| Field         | Value                                                        |
|---------------|--------------------------------------------------------------|
| ID            | ADR-0014                                                     |
| Status        | Proposed                                                     |
| Phase         | 0B                                                           |
| Created       | 2026-08-13                                                   |
| Last reviewed | 2026-08-13                                                   |
| Related       | P00B-T003, P00A-T001, ADR-0008, ADR-0016, ADR-0017, ADR-0034 |
| Supersedes    | —                                                            |
| Superseded by | —                                                            |

**Class: `Contract`.** It defines types, an encoding, a scope boundary, and error
behavior, so tests 1 and 3 of the [reversibility test](../ADR.md#the-reversibility-test)
both fail. Nothing about it is a value whose later change costs a rebuild: every
saved file would carry the old encoding.

## Context

The [identity inventory](../inventories/identities.md) records 31 identities and
references crossing the project, GUI, MCP, history, serialization, import, and
engine boundaries, after two audit passes. This record decides how a persistent
identity is generated, encoded, and scoped in Project Core V2.

Five properties of V1's model are load-bearing, and each is an entry in that
ledger.

- **Identity encodes type.** A module's identity is the string `osc-1`, and that
  is not a serialization convenience: the runtime type is literally
  `ModuleId { module_type: ModuleType, instance: u16 }`, whose own doc comment
  states the format. The persisted string is a faithful encoding of the engine's
  key (`IDN-0016`, `IDN-0029`).
- **Identity is load-bearing for audio.** `script_seed_base` folds **both** the
  type and the instance number into a per-(voice, module) PRNG seed, so
  renumbering a module *or changing its type* changes its script's random stream
  (`IDN-0029`). Identity is not merely referential.
- **Allocator cursors are persisted.** Six `next_*_id` counters live in the song
  and one `next_note_id` in each pattern (`IDN-0024`, `IDN-0025`). A counter
  below an existing id silently produces duplicates, and nothing validates it on
  load.
- **Scope is inconsistent.** `NoteId` is unique within a pattern, so it is not
  addressable alone (`IDN-0004`); module instance numbers are per *graph*, so a
  patch and the master effect chain allocate from different namespaces
  (`IDN-0021`, `IDN-0026`).
- **Width is inconsistent.** `InstrumentId(u64)`, `PatternId(u32)`,
  `TrackId(u16)`, `ReturnBusId(u16)`, `NoteId(u64)`, and `ModuleId.instance(u16)`
  — five widths for one kind of concept, with `u16` exhaustion an unhandled
  wrap in release builds (`LIMIT-0058`).

Around these sit the ordinary problems the ledger's own checklist names: raw
primitives where a newtype exists (`IDN-0011`..`IDN-0013`), positional
references (`IDN-0019`..`IDN-0023`), one load-time heuristic repair
(`IDN-0028`), and a display name used as identity at a service boundary
(`IDN-0031`).

**Outside this decision.** Parameter and port names (`IDN-0015`, `IDN-0017`,
`IDN-0018`) are a *closed vocabulary declared by a node type*, not entity
identities; ADR-0007 and the node contract own them. Asset and sample identity
(`IDN-0010`, `IDN-0030`) is ADR-0017's. What a track or bus *is* (`IDN-0002`,
`IDN-0005`, `IDN-0021`) is ADR-0034's; this record decides only how such a thing
is named. Unknown-field and enum-ordinal policy (`IDN-0014`) is ADR-0016's. How
a script's random stream is seeded once identity stops seeding it is ADR-0008's,
and clause 11 states the requirement this record hands it.

## Decision drivers

- **Merge and import must not need remapping.** The plan's Phase 10 imports V1
  projects, and tracker import already exists on a branch. Two documents whose
  identities are drawn from per-document counters cannot be combined without
  rewriting one side's references — the failure mode `IDN-0024` describes.
- **Fixtures must be byte-stable.** The reference corpus rebuilds its inputs
  from code and compares them to committed bytes. An identity scheme that draws
  randomly per entity makes every fixture regeneration a diff.
- **Identity must stop affecting audio.** `IDN-0029` is the reason the corpus's
  `shared-patch-or-instrument` category is blocked: a shared instrument's sound
  depends on how its two references are numbered. Until that is severed, a V2
  that assigns ids differently sounds different, and the corpus reports it as a
  regression.
- **The document must be self-consistent without a cursor.** A saved allocator
  position is state that can disagree with the data it allocates for.
- **An identity must be inert.** Nothing may parse it, order by it, or infer a
  type, a position, or a name from it.

## Options considered

### Option A: Document-scoped monotonic counters, as V1 has

Keep integer ids allocated from a counter per document, with the counter either
persisted or re-derived. Cheapest migration and smallest encoding, and the
inventory shows the re-derivation half already works: `add_module_with_id`
raises the per-type counter by `max()` when a loaded id exceeds it, which is a
sound reconciliation for a well-formed file.

It cannot answer merge. Two documents both start at 1, so every combination
needs a remap pass over every reference, and a remap pass is exactly where a
missed reference becomes a silently dangling one. It also keeps `IDN-0024`'s
duplicate-on-low-counter failure unless load validates the cursor.

### Option B: Random identity per entity — UUIDv4 or a random `u64`

Merge-safe with no coordination, and no cursor to persist. Widely understood.

It makes fixtures non-reproducible unless the generator is seeded, which turns
"random" into "seeded deterministic" and loses the property that motivated it.
It also gives up ordering: nothing about two ids says which was created first,
which the history and diagnostics surfaces would have to carry separately. And
collision safety is probabilistic per *entity*, so the bound scales with how
many entities exist rather than with how many documents do.

### Option C: Origin-namespaced monotonic identity

An identity is a pair: an **origin**, drawn once per document-creation event,
and an **ordinal**, monotonic within that origin. Two documents created
independently have different origins, so their identities cannot collide and a
merge needs no remapping at all. Within one origin, allocation is a counter, so
a fixture that fixes its origin is byte-reproducible.

It does not remove persisted allocation state, and an earlier revision of this
record wrongly claimed it did. A document has to carry the origin it mints from
and the highest ordinal it has ever minted, because deriving the next ordinal
from surviving content reissues the ordinal of a deleted entity. What it removes
is seven *unvalidated per-kind* cursors, replacing them with one record checked
against the document on load.

Costs a wider identity than either alternative, and puts a structure inside the
identity that a reader may be tempted to interpret. Its sharp edge is a document
copied outside the application: two files then mint from one origin with nothing
able to observe the copy, so the merge path has to detect the collision rather
than assume distinct origins.

### Status quo

No V2-specific decision. The project format keeps `osc-1`, identity keeps
feeding the PRNG seed, seven allocator cursors stay in the document, and Phase
10's import path has to invent a remapping rule per entity kind under deadline —
which is how `IDN-0028`'s load-time heuristic repair came to exist in the first
place.

## Evidence

- The [identity inventory](../inventories/identities.md) at `dd69b657`: 31
  entries over two passes, the second of which corrected two pass-1 hypotheses
  by reading the code rather than the schema.
- Source reads at `e2a05028` confirming the four claims this record leans on
  hardest: `synth_engine/src/commands.rs:29-33` (`ModuleId` carries the type),
  `synth_engine/src/graph.rs:24-28` (`script_seed_base` folds type *and*
  instance into the seed), `graph.rs:176-181` (per-type counter, increment
  only), `graph.rs:236-245` (`max()` reconciliation on load).
- Width survey at the same revision: `synth_core/src/types/identifiers.rs:20`,
  `synth_sequencer/src/ids.rs:32,636,666,679`.
- `LIMIT-0058` in the [resource inventory](../inventories/resource-limits.md):
  `ModuleId.instance` is a `u16` incremented with no ceiling check, so
  exhaustion panics in debug and wraps in release.

**A ledger entry this record found stale.** `IDN-0027` says undo re-adds a
deleted note under a *fresh* id, so identity does not survive a delete/undo
cycle. That is fixed at `e2a05028`: the `AddNote` arm calls `restore_note` with
the original note, and its comment states the reason. The entry needs
correcting, and this record does not depend on it either way.

**Uncertainty that remains.** No option here has been prototyped, and nothing
has been measured. Two questions the inventory itself leaves open bear directly
on the migration and are not answered here: whether module ids in
`global.master_effects` can collide with a patch's (`IDN-0021` records the
namespaces as different and the overlap as unchecked), and what the closed set
of parameter-name strings actually is (`IDN-0015`). Both are format-review work
that Phase 0B owes before this record is accepted.

## Decision

Proposed, not accepted. Thirteen clauses.

### The identity

1. **A persistent identity is an opaque pair** — an `Origin` and an `Ordinal`,
   both 64-bit — carried by one generic newtype per entity kind:
   `InstrumentId`, `TrackId`, `PatternId`, `NoteId`, `NodeId`, `BusId`,
   `GraphId`, and so on. **Distinct kinds are distinct types**; an identity of
   one kind may never be assigned to, compared with, or converted into another.
2. **An identity is inert.** Nothing may parse it, infer a type or a position
   from it, sort domain data by it, or reconstruct it from a display name. Its
   only operations are equality, hashing, and canonical serialization. This is
   the clause `IDN-0016`, `IDN-0018`, `IDN-0023`, and `IDN-0031` all violate.
3. **One width, everywhere.** The five current widths collapse to one. A kind
   that today uses `u16` gains range rather than keeping a narrower ceiling for
   compactness, and `LIMIT-0058`'s unhandled wrap ceases to exist.

### Allocation

4. **A document carries exactly one `AllocationRecord`**: the origin new
   identities are minted from, and the highest ordinal ever minted from it. Both
   are persisted. Every identity in the document may carry *any* origin — its
   own, or one inherited through a merge — but only the allocation origin ever
   mints.
5. **Ordinal is monotonic within an origin, and is never reused.** Deleting an
   entity does not free its ordinal, and the high-water mark in clause 4 is what
   makes that hold across a save and a reload.
6. **The allocation record is validated against the document, not trusted.** On
   load, no identity bearing the allocation origin may have an ordinal above the
   high-water mark; a document that violates this is **refused**, naming the
   offending identity. The record may legitimately sit *above* the highest
   surviving ordinal — that is precisely the deletion case — so the check is
   one-sided.
7. **Identity is document-scoped, never container-scoped.** A `NoteId` addresses
   a note without its pattern, and a `NodeId` addresses a node without its
   graph. `IDN-0004`'s "must always be paired with a `PatternId`" and
   `IDN-0026`'s per-graph namespace both go away.

   **A first revision of this record derived the next ordinal as `max(seen) + 1`
   with nothing persisted, and called that an improvement on V1's cursors.** It
   is not: deleting the highest-ordinal entity lowers `max`, so the next
   allocation reissues a retired ordinal and clause 5 is contradicted by clause
   6. The master plan forbids exactly that outcome — a reference must never
   silently point at the object that later occupies a reused slot. V1's own
   `max()` reconciliation, which that revision cited as prior art, carries the
   same hole; the ledger's "sound for a well-formed file" quietly assumes the
   highest id is never deleted. **What replaces seven unvalidated per-kind
   cursors is therefore one validated record, not nothing.**

### Forking and merging

8. **Forking mints a new allocation origin and remaps nothing.** Save As,
   duplicate, template instantiate, and project conversion give the new document
   a freshly drawn origin and a high-water mark of zero. Every existing identity
   keeps the origin it already had, so no reference is rewritten and the two
   documents can never mint colliding identities afterwards. This is the cheap
   half of the master plan's central duplication/remapping service: the
   remapping case remains for asset and cross-document references, but entity
   identity does not need it.
9. **A merge concatenates and never remaps.** The result's allocation record is
   the *receiving* document's, unchanged; the merged-in document's origins
   survive as identity namespaces that no longer mint.
10. **Two documents sharing an allocation origin may be merged only when one's
    ordinal range for that origin is an ancestor of the other's** — that is, no
    ordinal appears in both bound to different content. Otherwise the merge is
    **refused, naming the colliding ordinals**, because concatenating would
    silently alias two entities.

    **Shared origins are not confined to deliberately fixed fixtures**, which an
    earlier revision of this record claimed. Copying a file outside the
    application produces two independently editable documents with the same
    allocation origin, and clause 8 cannot intercept that because nothing
    observes the copy. Clause 8 removes the case for every in-application path;
    clause 10 is what remains for the paths outside it.

### The audio consequence

11. **Nothing derives audio state from an identity.** `script_seed_base`'s
    dependence on type and instance number is forbidden: a node's random stream
    is seeded from data the node carries, persisted explicitly, and preserved
    across renumbering. ADR-0008 owns what that seed is; this record fixes only
    that it may not be the identity. **The V1 conversion must carry the seed, not
    the id** — a converted project whose scripts sound different is a conversion
    defect, and `IDN-0029` is why the corpus cannot author its
    `shared-patch-or-instrument` case until this holds.

### Encoding and exhaustion

12. **The canonical serialized form is a string**, `<origin>-<ordinal>`, because
    JSON numbers cannot carry 128 bits and a two-field object multiplies every
    reference site in the format. The grammar is exact, and parsing is fallible:

    - `origin` is **exactly 16 lowercase hexadecimal digits**, zero-padded;
    - `ordinal` is **1 to 16 lowercase hexadecimal digits with no leading zero**,
      except that zero itself is the single digit `0`;
    - the separator is one `-`, and nothing else may appear.

    **A non-canonical spelling is rejected, not normalized.** Uppercase digits, a
    padded ordinal, an over-long field, and a trailing separator are all parse
    errors. One encoding per identity is what keeps a document's bytes
    deterministic and its digests meaningful; accepting two spellings of one
    identity would make equality depend on which one a writer chose. Construction
    is `TryFrom` at every external boundary, per the repository's newtype rules.
    Format version and unknown-field behavior are ADR-0016's.
13. **Ordinal exhaustion is a refusal, never a wrap.** An allocation that would
    take the high-water mark past `u64::MAX` fails with an error naming the
    document and its allocation origin; the remedy is a fresh allocation origin
    under clause 8, which costs nothing and remaps nothing. Reaching 2^64
    allocations in one origin is unreachable in practice, and turning the
    unreachable case into a refusal is what keeps the invariant total — the same
    treatment ADR-0032 gives `StreamEpoch` exhaustion.

**Deterministic authoring is a fixed origin, not a special mode.** A corpus
fixture or a generated example declares its allocation origin as a constant, and
its ordinals then follow from build order, so its bytes are reproducible. Two
fixtures must not share a constant; the generator asserts that across the set it
generates, and clause 10 is the backstop if one slips through.

## Consequences

### Positive

- Merge and import stop being a remapping problem, which removes the class of
  bug `IDN-0028`'s heuristic repair belongs to.
- Seven unvalidated per-kind allocator cursors become **one record that load
  validates against the document**, so the duplicate-on-low-counter failure
  nothing currently checks becomes a refusal that names the offending identity.
  The cursor does not disappear — an earlier revision of this record claimed it
  would, and that claim is what made the never-reuse guarantee unfulfillable.
- A module can change type without changing identity, and a plan can renumber
  nodes without changing what the project sounds like.
- The corpus's `shared-patch-or-instrument` category becomes authorable.
- One width and one shape make an identity conversion at any boundary either
  correct or a compile error.

### Negative

- **Every persisted reference in the format changes.** This is the widest
  breaking change in the project format, touching `IDN-0001`..`IDN-0009`,
  `IDN-0011`..`IDN-0013`, and `IDN-0016`..`IDN-0026`.
- Identities grow from 2–8 bytes to 16, plus their string encoding. Immaterial
  in a document; it is why the compiled plan uses compact indices instead.
- Ordinals are not dense, so nothing may use them as array indices — which is
  the point, and also a trap for anyone who assumes otherwise.
- The V1 conversion must carry per-node seeds it currently does not store, so it
  cannot be a pure syntactic rewrite.
- A document copied outside the application keeps its allocation origin, so two
  copies can mint colliding identities. Nothing in the format can prevent it;
  clause 10 detects it at merge, which is later than one would like.

### Risks and controls

- **Risk: the conversion drops a reference.** Every V1 id form appears in
  connections, groups, exposed ports, effect-chain order, Mod Matrix slot
  addresses, sends, automation targets, and the arrangement. Control: the
  conversion is driven from the identity inventory, and a round-trip fixture per
  entry class fails when a reference is dropped — the Phase 0B fixtures
  P00B-T005 begins.
- **Risk: the seed obligation in clause 11 is forgotten**, and converted projects
  quietly change sound. Control: a corpus case with a script whose output
  depends on its seed, converted and compared. It does not exist yet; the
  `yams-control-patch` category is where it belongs.
- **Risk: clause 2 erodes.** An opaque identity with visible structure invites a
  reader to parse it. Control: the string form is produced and consumed by one
  pair of functions, and no other code constructs it from parts.
- **Risk: the high-water mark and the document disagree** after a hand edit, a
  partial write, or a bad conversion, and allocation silently reissues a retired
  ordinal. Control: clause 6's one-sided load check, and a round-trip fixture
  that deletes the highest-ordinal entity, saves, reloads, allocates, and
  asserts the new identity is above the deleted one.
- **Risk: two copies of one document both mint**, which clause 8 cannot see.
  Control: clause 10's ancestor test at merge, plus a fixture that forks a
  document, edits both sides, and asserts the merge is refused with both
  colliding ordinals named.
- **Risk: a non-canonical spelling is accepted somewhere** and two strings
  denote one identity, which would make a digest depend on the writer. Control:
  one fallible parser, rejection tests for uppercase, padded, over-long, and
  truncated forms, and no second construction path.

## Follow-up work

| Task                                                                                          | Phase | Status      |
|-----------------------------------------------------------------------------------------------|-------|-------------|
| Correct `IDN-0027`: undo restores a note under its own id at `e2a05028`                       | 0B    | Complete    |
| Answer `IDN-0021`: can a master/return chain's module id collide with a patch's?              | 0B    | Not started |
| Establish the closed parameter-name set `IDN-0015` leaves open                                | 0B    | Not started |
| Fill the ledger's `Proposed V2 newtype/rule` and `Migration` columns from this record          | 0B    | Blocked on acceptance |
| Specify the conversion mapping per identity class, driven from the inventory                  | 10A   | Not started |
| Round-trip fixture: delete the highest ordinal, reload, allocate, assert no reuse             | 10A   | Not started |
| Fork-and-merge fixture: same-origin collision is refused, naming both ordinals                | 10A   | Not started |
| Rejection tests for non-canonical identity spellings, and for ordinal exhaustion              | 10A   | Not started |
| Persist per-node script seeds so clause 11 can hold (ADR-0008)                                | 7/10A | Not started |
| Author the corpus's `shared-patch-or-instrument` case, unblocked by clause 11                 | 0A    | Not started |

## Revisit conditions

- A merge or synchronization requirement appears that needs identities to be
  *comparable across origins* — a total order, or a happens-before — which the
  origin/ordinal pair deliberately does not provide.
- Measurement shows 16-byte identities to be a material cost in the document
  model or in an operation log, which would reopen the width in clause 3 rather
  than the model.
- The closed-vocabulary boundary in *Outside this decision* proves wrong: if a
  parameter name turns out to need entity identity rather than a declared name,
  clause 1's kind list is incomplete.
