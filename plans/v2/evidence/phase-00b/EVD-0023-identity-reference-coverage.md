# EVD-0023: Identity and Reference Audit Coverage

| Field | Value |
|---|---|
| ID | EVD-0023 |
| Status | Complete |
| Phase | 00B |
| Created | 2026-09-20 |
| Last reviewed | 2026-09-20 |
| Supersedes | — |
| Superseded by | — |
| Source revision | `c4bb53f8f15d70debc14a2e438d3925b66777d2d` (audited schemas and Rust source) |
| Retention | Permanent |
| Related | P00B-T003, IDN-0001–0042, ADR-0014 |
| Artifacts | `EVD-0023-schema-review.json`, `scripts/check_identity_coverage.py`, `scripts/test_check_identity_coverage.py`; the identity ledger's conversion table |

## Question and falsifier

Can P00B-T003 close as an audit of existing identities and references, each with a proposed V2 rule and a conversion
or named deferral disposition? This is not a test of a converter or acceptance of a new format. The master plan's
identity inventory bullet asks for a **proposed rule**; its later contract and phase-exit bullets remain separate.

The conclusion is Supported only if the schema review covers every declaration, all ledger entries have nonblank
fields and one conversion disposition, and independent source review finds no omitted identity/reference class,
false ownership claim or unowned first-consumer obligation. A contrary source declaration blocks completion even
if the mechanical check passes. Optional destination implementation details do not block this source audit.

The mechanical falsifier is schema drift without a matching reviewed declaration, an undefined/uncovered ledger
entry, or an incomplete/duplicate conversion disposition. Negative controls mutate the real input copies before
the unmodified repository run: add a field to an existing definition, add/remove a definition, change a map's key
contract, alter a tuple endpoint, delete a patch definition, remove a claim or conversion row, and duplicate an ID.
Each must produce its named diagnostic. The method does not detect a semantically incorrect classification whose
fingerprint has been deliberately updated; that requires the independent reader.

## Inputs and controls

The committed source revision above is the subject. The uncommitted audit adds only the classifications, checker,
controls and this record; its final version is retained in the commit containing this record. There is no timed
comparison, DSP experiment, converter implementation or synthetic success model.

- Full `schemas/project.schema.json`, `patch.schema.json` and `bundle-metadata.schema.json`, including nested union
  branches, tuple endpoints, `patternProperties` keys and the open Rust parameter/script maps.
- Declaring Rust types and the actual source consumers below. A schema alone cannot describe dynamic YAMS knobs,
  raw-string Mod Matrix references or runtime-only IDs.
- Every identity ledger row and its conversion disposition; prior dated source audits remain explicitly bounded.
- Negative controls use deep copies of those actual inputs; no repository schema is modified by a control.

## Method

Walk every schema root and definition. Review members, variants, map keys and referenced declarations; assign each
whole declaration either to its relevant identity ledger entries or explicitly to value data. Referenced definitions
are reviewed separately; a container also names the entries explaining its own reference fields (for example,
ModGraph lists graph/node identity and the assigned-track reference). It does not duplicate all descendant claims. Retain a SHA-256 of each complete
canonical JSON declaration. The checker rejects new, missing or changed declarations, not just a changed count.
It also compares the standalone patch with the project's Patch shape and matching definitions.

A declaration may relate to several concepts: an InstrumentState declares an instrument, holds a sidechain reference,
and contains a MIDI selector. This is **declaration coverage**, not an exactly-once partition of every property. The
ledger defines each concept once; its composite-reference entries refer to the existing target-identity concepts.
A newly discovered reference requires semantic review even when its target's type already has an entry.

The reader identifies concept and conversion rows only inside contiguous tables with the exact header and a Markdown
separator. A blank line ends a table; similarly shaped rows elsewhere cannot define an ID or satisfy a disposition.
Classified and Verified entries require complete fields and a conversion rule/verification owner. Discovered,
Investigating and Needs review remain valid reopening states; the checker reports their status instead of calling
them Classified. An audit closure still requires the semantic review and completion conditions above.

The source audit follows these boundaries:

| Surface | Declarations and consumers inspected | Identity coverage |
|---|---|---|
| Project and patch entities | `pertylizer/src/project.rs`, `patch.rs`; project schema and standalone comparison; prior session/project_apply traces | IDN-0001, 0009, 0011–0014, 0016, 0019–0022, 0026 |
| Song/pattern/note allocation, bindings and copy | `synth_sequencer/src/ids.rs`, `song.rs`, `pattern.rs`, `note.rs`; prior undo_flow trace | IDN-0002–0008, 0023–0025, 0027 |
| Mod/Note graph maps and embedded references | `synth_sequencer/src/mod_grid.rs`, `note_graph.rs`, `automation.rs`; keys of nodes, descriptions and positions, endpoint fields, assigned_tracks, AutomationTarget and AudioTapSource variants | IDN-0006/0007, 0015/0017, 0034/0035 |
| Ordered records without IDs | PatternPlacement, AutomationLane/Point, TrackSend/ReturnSend, connections, legacy processors, tempo/time-signature events | IDN-0036; positions locate source occurrences but must not become durable entity IDs |
| Parameter/address payloads | `patch.rs::ParamValue`, upgrade_legacy_mod_matrix; `synth_core/src/params/mod_matrix.rs` SrcAddr/DestAddr; session script application | IDN-0010, 0015, 0018, 0028, 0040 |
| Assets | `pertylizer/src/bundle.rs` metadata and archive lookup, `synth_sampler/src/library.rs`; prior duplication trace | IDN-0010/0030; filename and metadata are two representations of one sample identity |
| Runtime and randomness | `synth_engine/src/voice.rs`, `voice_allocator.rs`, `graph.rs`; `synth_sequencer/src/note_graph.rs`, `note_processor.rs`; workspace searches for HostKey/NoteEventKey | IDN-0029/0037; runtime salts are not persisted entity IDs |
| Declared vocabularies and routes | ModuleType, parameter/port descriptors, InstrumentState.channel, MidiCcNode and AudioTapSource | IDN-0014/0015/0017/0038; typed selectors remain distinct from project entities |
| Existing experimental lowering | `pertylizer/src/lowering/identity.rs`, `modulation.rs`, `buses.rs`, tests.rs | IDN-0039; current bounded arithmetic addresses are not a 128-bit persistent-ID adapter |
| Session revisions and history targets | `synth_core/src/types/revision.rs`, `synth_sequencer/src/shared_song.rs`, `song.rs`; `pertylizer/src/dirty.rs`, `gui/egui_backend.rs`, `undo.rs` | IDN-0041/0042; session counters, coalescing and widget seeds are not canonical project identities |
| Public/service/corpus boundaries | Prior 2026-09-19 audit of hub.rs, transactions.rs, exports and MCP results; render/mix.rs and corpus/mod.rs | IDN-0031–0033; no claim that unknown external consumers are absent |

All relative source paths above are under `crates/`. These are source observations, not executed V1 load/copy tests.
The source scan found nine classes not explicit in the previous ledger; IDN-0034–0042 now give each a rule and
consumer. Specifically, a ModTarget's embedded AutomationTarget and an AudioTap's track are not covered merely by
remapping a Mod Graph's assigned_tracks. Graph metadata map keys also require remapping even though the payload is
only a string or position.

The `NoteEventKey` workspace search found its declaration, export and tests, but no production caller. HostKey folds
local host IDs; note expansion calls it with NoteId without a pattern qualifier. The fallback seed includes a slot.
This audit therefore does not claim that V1 note randomness is stable under arbitrary reordering or migration.
`ModuleState.scripts` is a separate slot-keyed map, and current session loading can diagnose and skip unsupported
slots. A future supported conversion must not silently turn that behavior into complete migration.

The reread also corrects IDN-0028: V1's `positional_instances` sorts instance suffixes numerically within each
module type before resolving a legacy role index. File-array order is not the lookup order. Conversion must preserve
that original owner/type-qualified role meaning, and cannot use a same-spelled module from another chain.

CorpusCaseId/BehaviorClaimId are evidence-manifest identities, not project-conversion inputs. Their existing corpus
validation remains authoritative. Diagnostic paths and GUI widget keys are locators/presentation keys built from the
entities above, not additional persistent entity allocations. No conversion rewrites evidence IDs or widget seeds.

## Reproduction

Run from the repository root:

```text
python3 -B -m unittest scripts/test_check_identity_coverage.py
python3 -B scripts/check_identity_coverage.py
python3 -B scripts/check_v2_docs.py --evidence
python3 -B -m unittest scripts/test_check_v2_docs.py
rg -n 'HostKey|NoteEventKey|fallback_note_seed|VoiceId' crates --glob '*.rs'
rg -n 'AutomationTarget|AudioTapSource|scripts|assigned_tracks|node_positions|node_descriptions' crates/synth_sequencer/src crates/pertylizer/src/patch.rs
```

## Results

The negative controls passed before the baseline: 23 identity-coverage tests, including the real-input mutations
and the unmodified repository check. The baseline reports **137 reviewed schema declarations** (122 project,
11 standalone patch, 4 bundle) and **42 Classified entries**, each with one conversion disposition.

| Check | Observed result |
|---|---|
| Added field, added/removed definition, changed graph map key or tuple endpoint | Each returns the expected changed/unreviewed/stale declaration diagnostic |
| Deleted standalone definition or a refreshed-but-divergent patch root | Rejected by coverage or structural comparison respectively |
| Removed runtime claim, undefined/duplicate ledger ID, blank migration field | Each returns its expected coverage/classification diagnostic |
| Removed/duplicated conversion disposition, missing classification reason | Each returns its expected completeness diagnostic |
| Broken table, missing separator, unrelated eleven-column table or conversion row outside its table | Cannot define the missing concept or satisfy its disposition |
| Blank conversion rule | Rejected for a completed classification |
| Verified or Needs review transition | Accepted as valid vocabulary; incomplete migration is allowed for a reopened entry |
| Unknown status | Rejected |
| Original inputs | No diagnostics; project and standalone patch agree |
| Documentation integration | The failure-propagation test verifies that an identity-checker error reaches the documentation gate; all 36 documentation-checker tests pass |
| Evidence gate | `check_v2_docs.py --evidence` passes, including the existing EVD-0016 simulator and both inventories' controls |

No Rust behavior was changed and no new runtime equivalence observation is claimed. The independent semantic review
required by AGENTS.md evaluated the task-closure claim and confirmed the source premises. It found a broken table,
a shape-only parser, a Classified-only status restriction and a missing ModGraph-to-track review-map link. Those were
repaired with table-scoped parsing, vocabulary-aware completeness checks and the missing link; negative controls
cover the parser/status repairs. The evidence-method repair receives a focused independent reread before commit.
The test module can also be executed by absolute path outside the repository; its imports no longer depend on cwd.

## Limitations

- The schema guard detects drift in a reviewed surface. It cannot discover an undocumented serialized field, prove
  dynamic YAMS declarations correct, or interpret arbitrary parameter strings. Rust/source review supplies those
  classifications. Existing empty closed script-parameter schema maps remain a known defect, not proof of no knobs.
- Runtime/service source is not fingerprinted. A new source surface or changed ownership needs a new audit; the
  register's Current status is bounded by its source revision, not a promise about unreviewed future code.
- No converter, V2 envelope, copy operation, allocator or history implementation is tested here. No entry becomes
  Verified. The conversion table states required rejection tests and first consumers, not observed passing migrations.
- V1 entity zero is not universally absence; V1 allocation/deletion history and random laws cannot be reconstructed
  by preserving the old integer. Independent conversions have no claimed equal-ID, equal-byte or audio-parity result.
- Public API removal, persisted-format breaks and the unmerged tracker-import work remain outside this approval.

## Conclusion

**Supported for the bounded inventory outcome.** The source audit and guarded schema review provide the proposed
rules, explicit conversion cases and named deferrals needed to close P00B-T003. The ledger is Current against the
named source, and all 42 rows are Classified. They are not Verified migrations. A later omitted reference or false
source premise reopens the affected entries and this conclusion; adding an unrelated implementation detail does not.

ADR-0014 remains Proposed, with no format or API change approved by this record. P00B-T006/T007, canonical identity
implementation, history, format round trips and external-service decisions retain their explicit first-consumer gates.
Phase 0B itself has not exited.
