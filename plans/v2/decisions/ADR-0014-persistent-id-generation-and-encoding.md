# ADR-0014: Persistent ID Generation and Encoding

| Field | Value |
|---|---|
| ID | ADR-0014 |
| Status | Proposed |
| Phase | 0B/10A |
| Created | 2026-08-13 |
| Last reviewed | 2026-09-20 |
| Related | P00B-T003, P00A-T001, ADR-0008, ADR-0016, ADR-0017, ADR-0034 |
| Supersedes | — |
| Superseded by | — |

**Class: `Contract`.** Acceptance would bind persisted identity, copying and
conversion across phases. This revision is a proposal, not approval to change
an existing file format, public API, manifest or protocol. No runtime behavior
changes with this document; accepted ADR-0008 remains authoritative for scripts.

## Boundary and readiness

Phase 0B needs a coherent stable-ID proposal before its current contract can be
written. The previous draft conflated copying with forking, allowed conflicting
retained identities to merge, and prohibited a seed input accepted by ADR-0008.
This revision replaces those claims together. It keeps the existing thirteen
clause numbers for inventory references.

The scope is persistent entity identity and the identity part of authoring
operations. Parameter/port vocabularies, assets, editor-state ownership,
track/channel ownership and transaction results keep their named owners. In
particular, IDN-0032/0033 are service/operation identities, not persisted entities.
This record supplies no generic merge of project settings, asset contents,
history, live VM state or two edited versions of one entity.

## Evidence and factual premises

The [identity inventory](../inventories/identities.md) owns the source audits:

- IDN-0004/0025: V1 note identity is local to a pattern; conversion lookup must
  qualify the old ID by its pattern.
- IDN-0006/0007 and the 2026-09-08 inspection: graph duplication copies local
  node IDs and internal references. Document-wide node identity needs explicit
  remapping of that owned content.
- IDN-0016/0021/0026: V1 module IDs encode type and are owner-local. Conversion
  keys must include the owning patch or effect chain; deleted allocator history
  cannot be reconstructed from surviving modules.
- IDN-0024/0027: allocator state and restoring an existing identity are separate
  from creating a new entity. Undo must restore the original identity.
- IDN-0029: V1's script seed depends on its module identity. This does not prove
  any conversion preserves V1's random stream.

At `a83cd92e`, [the experimental IR](../../../crates/synth_engine_v2/src/ir.rs)
uses `NodeId(u32)`. [ScriptIdentity and ScriptProgram::seed](../../../crates/synth_engine_v2/src/script.rs)
retain that node identity and combine project seed, node identity and script-state
identity; `ScriptSeed::for_voice` adds stable voice identity. These source facts
agree with [ADR-0008](ADR-0008-yams-state-identity-and-seeds.md). They do not provide
a persisted 128-bit identity implementation. No copying, merge or conversion
experiment was executed for this proposal.

## Options and rationale

- **Owner-local counters:** small, but require owner-qualified references and
  remapping when content is combined. Surviving maxima alone lose deletion history.
- **Random per-entity IDs:** viable with collision checks and a controlled fixture
  generator; they do not themselves solve fork conflicts or copy semantics.
- **Origin plus monotonic ordinal (proposed):** one active allocator for all entity
  kinds, with retained origin history. Fixtures can supply a fixed origin. Random
  origins are not a mathematical uniqueness proof; validation and refusal remain
  necessary.

The proposal chooses fresh identities for independent copies, preserved identities
for document forks, and a deliberately limited identity-preserving import between
**disjoint origin histories**. Overlapping histories refuse even if their surviving
entities look equal. Content equality cannot prove ancestry or distinguish a
retired identity from a colliding allocation in an independently edited file.
An explicit copy/import-as-copy remains available once its reference bindings are
complete. No operation silently falls back from preserving identity to copying.

For script determinism, the proposal follows ADR-0008's stable node identity as a
seed input. It withdraws the earlier blanket demand that identity never affect
audio. New independent copies may produce different random output; rearranging
storage or recompiling the same identities must not change their seed inputs.
The user selected independent deterministic random sequences for duplicates on
2026-09-19; this does not approve the separate persisted-format/API break.

## Decision

Proposed, not accepted. The rules below are one candidate contract; acceptance
and implementation requirements are listed under the retained anchor below.

### The identity

1. **A persistent entity ID is an opaque `(Origin, Ordinal)` pair**, both 64-bit,
   with distinct domain newtypes such as `InstrumentId`, `TrackId`, `PatternId`,
   `NoteId`, `NodeId` and `GraphId`. Kind is carried by the type/schema, never
   inferred from the pair. Entity ordinals start at 1; zero is reserved for an
   empty allocator high-water mark, not a valid entity ID.
2. **Identity is independent of type, name, ownership and display/storage order.**
   Application behavior does not parse IDs to derive those concepts. Equality,
   checked reference resolution, hashing and canonical serialization are allowed.
   Canonical map ordering is allowed but is not musical processing order. Using
   the full stable node identity as a deterministic seed input follows clause 11;
   a compact execution slot, display index or declared parameter key is separate.
3. **Persistent entity kinds use the same width.** This does not turn local
   parameter/port keys, tracker lanes, counts, runtime slots or service IDs into
   entity IDs. The owning canonical schema declares which fields are entity
   identities, owned children, references or intentional order values. A copy or
   conversion consumer with an unclassified field is refused before publication.

### Allocation

4. **One active origin, retained history for every origin.** Proposed
   `AllocationRecord` contains a typed active origin and a canonical map from
   `Origin` to the highest committed ordinal for that origin. The active origin
   is present even at high-water zero. All other entries are retired for allocation
   in this document, including origins brought in by a fork or import. They never
   become active again. The map is retained when entities are removed; it is not
   rebuilt from survivors. This replaces per-kind cursors, not all allocation state.
5. **Committed ordinals are never reused for new entities.** One checked counter
   under the active origin serves all persistent entity kinds. Delete, undo/redo,
   history truncation and save/reload neither reduce high-water marks nor remove
   known origins. Undo can restore the same entity and ID through retained history;
   it cannot reuse the ID for a new entity. A failed staged operation exposes no
   candidate IDs and publishes neither content nor allocator changes. Private
   candidates are not committed identities and may be discarded.
6. **Validate the complete allocation/reference boundary.** Every entity's origin
   must be recorded, and its ordinal must be positive and no larger than that
   origin's high-water mark. Duplicate pairs, even across kinds, are refused.
   Every required reference resolves to the declared kind and permitted owner;
   explicit absence is allowed only where its field contract permits it. Map
   keys are unique and the active origin exists. Missing, unknown or invalid
   representation fields fail under the eventual versioned schema. A bound below
   an existing ordinal is refused, not repaired by guessing a new cursor.
7. **Entity identity is document-wide, not container-local.** A note or node can
   be addressed without its pattern or graph, but authorization and ownership
   checks still apply. V1 conversion maps `(old kind, old owner, old local ID)`
   to a fresh typed identity. It cannot collapse equal local numbers or module
   strings from different owners. Restoration is an operation on known retained
   identity, not another route to allocate into a retired origin.

Origin generation uses an injected system-random source in normal authoring and
explicit origins in deterministic fixtures. A candidate equal to any retained
origin is rejected. The eventual implementation bounds retries and origin-history
storage and reports exhaustion/entropy failure before changing the document.
It cannot discard history to meet a capacity limit. No cross-document uniqueness
is assumed merely because a random draw succeeded locally.

### Forking, copying and merging

8. **Fork and copy are different operations.**

   **Document fork:** retain the entity content, all entity IDs, allocation history,
   project seed and local script-state/parameter identities. Add a fresh origin
   with high-water zero and make only it active in the new document. The source
   is unchanged. This is the identity policy for a new project fork, not a choice
   about Save As path handling, envelope/revision identity or recovery association;
   those belong to their application/format owners. No running VM state is copied.

   **Independent copy, including template instantiation/import-as-copy:** select
   roots and the complete transitive closure of their owned entities from one
   immutable source snapshot. Stage fresh destination IDs for every entity in that
   closure. Build one injective old-to-new mapping before rewriting anything.
   Remap ownership, connection endpoints, group membership, node-keyed metadata
   and every internal reference through that map, preserving explicit array order.
   A repeated reference maps to one copied entity; shared referenced entities do
   not become owned children merely because they are referenced.

   For an in-document copy, references outside the closure retain their existing
   targets after kind/ownership validation. This includes external track bindings;
   duplicating an assigned graph may therefore add another active contributor.
   For a cross-document copy, every external reference requires an explicit typed
   destination binding, or an explicit absent value where the field permits it.
   Equal names, numbers or content are never automatic bindings. Missing mappings
   and unsupported asset/editor/reference policies refuse the whole operation.
   Existing references outside the copy still point to the originals.

   Copy the script's local state identity and parameter namespace, including removed
   key reservations, under its **new** node identity. The destination project seed
   applies. Commit the rewritten content, allocation changes and returned mapping
   together; any collision, unresolved reference or resource failure leaves the
   destination unchanged. A copied root must be attached by the owning application
   operation before the result is valid; this rule does not invent attachment/order
   behavior for an undecided entity kind.

9. **Identity-preserving import requires disjoint retained origin histories.**
   This is an additive import of selected entity content, not reconciliation of
   two project versions. Validate source and destination snapshots first. Compare
   **all keys of both allocation histories**, including origins with no surviving
   entities and the active origins. A nonempty intersection refuses the operation,
   naming the overlapping origins. It does not compare payloads, deduplicate,
   overwrite, resolve a deletion or infer ancestry. The check catches forks with
   different active origins but shared retained identities as well as external file
   copies that kept the same active origin. It also conservatively refuses harmless
   overlap; that limitation is deliberate.

   With disjoint histories, preserve the selected entities' IDs and their internal
   references. Bring in the source's entire origin/high-water map as retired
   allocation history, even for a partial import. Keep the destination's active
   origin and its high-water mark unchanged. External references and attachment
   require the same explicit checks as cross-document copy. Destination project
   settings and project seed remain the destination's. Publish the complete result
   atomically only after combined validation. Re-importing from that lineage will
   refuse; the caller may explicitly request import-as-copy instead.
10. **Refusal is the conflict policy for shared origins in this first contract.**
    No same-ID content deduplication, three-way merge, last-writer-wins or deletion
    resurrection is inferred. Copy/import-as-copy allocates new entities rather
    than pretending a conflicting entity is the same object. A future true version
    merge needs its own ancestry, deletion and conflict contract before relaxing
    clause 9. History restoration within one document remains clause 5's distinct
    operation and cannot be requested by supplying an arbitrary external snapshot.

### The audio consequence

11. **Preserve seed inputs when identity is preserved; copying creates new inputs.**
    For Control and Audio, ADR-0008 remains the runtime law: project seed, full
    stable node identity, stable voice identity and explicit script-state identity
    participate in seeding. The Note domain follows
    [ADR-0060](ADR-0060-bounded-note-yams-source.md)'s authored-occurrence input;
    assigned voice and list order are not its seed inputs.
    Reordering declarations or assigning different compact execution slots changes
    none of those inputs. A document fork preserves them for corresponding nodes
    and voices. This promises the same initial seed inputs, not a copy of running
    VM history or identical output under different events, routing or programs.

    Independent copies get new node identities; preserving their old random stream
    is not promised. Identity-preserving import retains node identity but adopts
    the destination project seed, so it also makes no sound-preservation promise.
    Different input identities do not mathematically guarantee distinct finite PRNG
    seeds. Script-state identity is local to its node for copying; it is not a reason
    to retain the copied node's persistent entity ID.

    The canonical-to-Sound-Core consumer in Phase 10A must carry the **full pair**
    as the logical node identity, separate from compact execution slots. The
    existing experimental `NodeId(u32)` is insufficient: truncation, hashing into
    a compact *identity*, and numbering by declaration order are not adapters.
    Extend the experimental logical identity representation and deterministic seed
    mixing to consume both components before enabling this consumer, with tests
    for equal ordinals under distinct origins and recompilation/reordering. Hashing
    the full typed inputs to obtain a PRNG seed remains allowed. This document does
    not change that Rust API or its current numeric mixer, freeze new seed bytes,
    or claim cross-version experimental bit equality.

    V1 script conversion remains a separate fidelity obligation. The converter
    must diagnose an unrepresented random-stream law and exclude a parity verdict
    under the [lowering contract](../specs/spec-project-lowering-and-fidelity.md).
    Neither preserving V1's old module number nor this new ID scheme establishes
    that its output is preserved. No per-node seed storage or seed-preserving copy
    mode is introduced by this proposal.

### Encoding and exhaustion

12. **One canonical string encoding:** `<origin>-<ordinal>`. Origin is exactly 16
    lowercase hexadecimal digits; ordinal is 1–16 lowercase hexadecimal digits
    without leading zero. Entity ordinal zero is rejected. The high-water value
    uses the same ordinal grammar but allows the single digit `0`. Origin-map keys
    use the origin grammar. Duplicate keys and noncanonical spellings are rejected,
    never normalized. Canonical map order is ascending numeric origin, with entity
    sets ordered by their typed IDs only for serialization; authored ordered lists
    keep their explicit order. Exact envelope field names and version belong to
    ADR-0016/Phase 10D; no existing format version gains these meanings.
13. **Exhaustion refuses, never wraps.** Allocation past `u64::MAX`, unavailable
    fresh origin, or a resource limit fails before publication. An explicit
    allocator rotation can retire the active origin and add a fresh one at zero
    without remapping entities, using the same origin-history rules as a fork.
    It does not clear history or bypass any storage limit. Deterministic fixtures
    use explicit distinct origins; production never substitutes a fixture origin
    after random-source failure.

## Open acceptance questions

The three defects identified before this revision have proposed resolutions:

| Question | Proposed resolution | Falsifier |
|---|---|---|
| Copy inside a document versus fork | Clause 8 allocates and remaps the owned copy closure; a fork retains entities | Editing a copied node changes the original, or an internal copied reference targets the original |
| Fork/edit/merge across active origins | Clauses 4–6 retain origin history; clauses 9–10 refuse any overlap | Differently edited retained entities are silently deduplicated or overwritten, including after deletion/save/reload |
| Identity versus script seed | Clause 11 follows ADR-0008 and makes changed copy inputs explicit | A compact slot or truncated ID substitutes for stable identity, or unrepresented V1 randomness receives a parity verdict |

These are **design counterexamples and future checks**, not executed evidence.
This proposal can be reviewed without implementing Project Core. Acceptance still
requires explicit approval of the intended persisted-format/API break before its
implementation; approval to work on Phase 0B does not itself grant that break.
The exact format envelope, supported ownership schemas and conversion coverage
remain with their named owners, and block their first consumers. They do not
license a partial copy, an ambiguous reference or a silent format reinterpretation.

## Consequences and verification obligations

- The allocator is one active counter plus retained per-origin high-water history,
  rather than the previous draft's single origin record. That history has a bounded,
  explicitly refused growth path and survives undo/deletion. It is not a render
  allocation or audio-thread operation.
- Copying requires a complete typed reference traversal. Identity-preserving import
  avoids entity remapping only for disjoint histories and may refuse benign overlap.
- New entities may sound different when scripts use randomness. Retaining a node
  preserves a seed input, not a universal equivalence or migration verdict.
- Acceptance would change persisted entity encodings. No current schema or reader
  is modified here, and no migration disposition is marked verified.

Before implementation is enabled, the owning phases must demonstrate:

| Owner / check | Required observation |
|---|---|
| 10A allocation and 10C history | Delete highest ID, undo/redo, truncate history, save/reload, then allocate: no new entity receives a committed old pair; undo restores the original |
| 10A copy traversal | Graph copy remaps nodes, edges, groups and metadata; original external references stay put; unresolved external bindings refuse without any content/allocator mutation |
| 10A preserving import | Disjoint histories combine with source origins retired; shared active or retained origins refuse even when payloads are equal or all shared entities were deleted |
| 10A failed operations | Exhaustion, entropy failure, mapping errors and history-capacity limits publish neither partial content nor candidate IDs |
| 10A Sound Core bridge | Full origin/ordinal identity reaches node lookup and seed inputs; changing slot/order does not change inputs; distinct-origin equal-ordinal entities never alias |
| 10A/10D V1 conversion | Owner-qualified mapping covers every supported ID/reference; unsupported seed/asset/ownership semantics produce named refusals, not parity claims |
| 10D format | Round trips preserve allocator history and references; invalid spellings, zero entity ordinals, missing/duplicate/unknown fields and unapproved old-version input are rejected |

No new copy/import surface is enabled until its owning schema, limits, operation
result and tests exist. Phase 0B's identity audit is recorded in
[EVD-0023](../evidence/phase-00b/EVD-0023-identity-reference-coverage.md);
format/operation work remains open. This proposal is not the phase exit.

## Revisit conditions

- The product needs repeated identity-preserving import or reconciliation of two
  forks rather than explicit copy: decide ancestry/deletion/conflict semantics.
- The product needs seed-preserving independent copies: decide a separate random
  identity policy with ADR-0008's successor before promising it.
- Measured identity/history storage costs require a different representation;
  do not recover space by silently forgetting retired origins or ordinals.
