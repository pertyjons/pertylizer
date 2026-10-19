# ADR-0087: Native voice storage relocation

| Field | Value |
|---|---|
| ID | ADR-0087 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-10-18 |
| Last reviewed | 2026-10-18 |
| Related | ADR-0010, ADR-0048, ADR-0083, ADR-0084, ADR-0085 |
| Supersedes | ADR-0085's fixed voice storage and numeric mixer-row requirements for this consumer |
| Superseded by | — |

## Decision

Extend isolated compiled `ContinueOrReset` to an unchanged logical native voice program
whose state, history, prepared records, parameter rows and audio buffers occupy
different physical addresses. Keep logical playable indices, authored voice
identities/scopes, local instance indices, tuning, complete ordered note and automation
program, producer partition, device geometry and output latency. The actual minter and
live registry keep their table, generations and issuers; only their checked plan label
changes. No identity snapshot becomes authority.

Pair non-global steps in execution order. Each pair has the same kernel, prepared native
configuration, local/shared-sum role, layout and in-place policy. Every authored voice
identity anchors its first instance to the corresponding pair; instance roles and
contiguous authored control groups retain every other instance. Compiler-inserted
conversion and fade/sum steps also match. Compare inputs by their last preceding writer,
rather than by arena slot equality, and require corresponding voice producers in the
same input positions. Schedule order is a witness of the unchanged internal program,
never persistent identity. Relocated voice programs refuse global input dependencies.
The existing fixed physical case permits a global source only when its stable authored
identity and scope still match. Prepass extent stays unchanged; modulation, feedback,
controllers, scripts, samples, sends and compensation remain outside this case.

Translate instance/sum groups through the node bijection. Resolve voice controls through
authored node/parameter identity and retain each row's instance offset; compare all row
metadata and base bits. Note targets and their ordered magnitude fan-out match after
node, control-group and tuning rebinding. Logical note slots keep their exact indices
and authored identities. Global structural matching, fresh additions and retired
removals retain ADR-0085's rules. Existing channel/bus records rebind their controls
through stable authored node/parameter addresses; their remaining fields stay exact. Tap
addresses keep the same authored node/port and checked logical index. Tap payload
geometry comes from the successor plan, never from copied old node/buffer indices.
Unsupported shape chooses reset before structural resource limits using allocation-free
scans; admitted metadata and actual capacities fit the transition allowance and record
bound.

At B, validate both renderer origins, carry/freshness, the actual live registry, all
global and voice extents, and strictly increasing source and destination node/parameter
maps before mutation. Copy small state and parameter records and exchange history owners
into the prepared renderer. Then exchange prepared geometry into the actual renderer.
Callback work traverses admitted flat maps without allocation, destruction, locking or
topology resolution. Retired geometry stays in the owning return envelope. First
successful fragment, interrupted custody and optional ADR-0084 observed handoff retain
their existing rules.

## Falsifiers and limits

Acceptance fails on changed audio through neutral relocation, lost warm voice history or
automation, wrong later bend/release, lost in-progress stealing state, partial mutation
on refusal, duplicated destinations, callback allocator activity, identity-authority
replacement, lost owning credits or uncharged map capacity. Compiler controls add/remove
disconnected global nodes that actually shift voice state and parameter addresses. They
compare an unedited normal renderer at four rates and varied callback partitions, retain
a warm voice delay, repeat relocation in both directions and exercise invalid maps and
changed voice meaning.

The first repeated relocation asserts render clock 192 after 256 served frames because
output preparation supplies one priming quantum. It asserts an active steal fade before
and after installation. The control exposed a pre-existing stereo sum defect: copy spent
fade steps per channel, while accumulate also addressed positioned controls by
interleaved sample index. Both now take controls and spend one gain step per frame,
sharing that gain across channels. A direct two-quantum kernel oracle checks an
eight-frame fade beginning at frame 62, its remaining state and reset at frame 72 in
mono and stereo for both kernels.

This consumer does not relocate logical playable indices or reorder/change the voice
program. Added note destinations, changed polyphony, cross-boundary global inputs during
relocation, general mixed/live state migration and threaded subscriber lifecycle remain
separate work. Application graph-lineage and song-end fences still apply. The session
GUI/MCP publication remains session-local; Phase 0B/10A still owns canonical project
revision. No production or physical timing qualification and no Phase 9 exit is claimed.
