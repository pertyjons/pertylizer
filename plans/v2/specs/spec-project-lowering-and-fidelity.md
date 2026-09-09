# SPEC: Project Lowering and Fidelity Contract

| Field            | Value                        |
|------------------|------------------------------|
| Status           | Current                      |
| Phase            | 4                            |
| Created          | 2026-09-02                   |
| Last reviewed    | 2026-09-04                   |
| Based on         | ADR-0057, ADR-0056, ADR-0025 |
| Invariant prefix | LOWER                        |
| Supersedes       | —                            |
| Superseded by    | —                            |

Allowed status values are defined in [../specs/README.md](README.md). Only a `Current`
specification constrains implementation.

## Scope

What the V1-to-V2 project lowerer must do: what it produces for a saved project it can
represent, what it produces for one it cannot, and what may be claimed about the result.
It governs `crates/pertylizer/src/lowering/` and every current or future consumer of a
lowered outcome.

This contract exists because
[`spec-sound-core-render-contract.md`](spec-sound-core-render-contract.md) deliberately
excludes project lowering, while the rules below bind implementation now.

## Non-goals

It does not define the V2 render plan, the compiler, event scheduling or the note payload —
those are the Sound Core render contract's, and `SOUND-INV-021` owns the payload's
magnitudes and, since ADR-0059, how V1's two velocity sensitivities compose; the lowerer
carries each saved sensitivity to its destination and decides nothing about them. It does not define
a comparison harness, a shared render request or a job contract; ADR-0028 owns those and
Phase 10B accepts it. It does not decide which V1 module types are supported, which is a
per-phase subset question rather than a contract.

## Terminology

- **Lowering** — the pure transformation from a saved V1 project into a V2 `ProjectGraphIr`
  and its events. It reads a project, never a file, and never the live engine.
- **Outcome** — a lowering's result: its plan or refusal, its diagnostics, and the counts
  taken while producing them.
- **Fidelity** — whether an outcome represents everything the project asked for.
- **Parity verdict** — any claim that a V2 render reproduces V1's for a corpus case,
  including a per-claim judgement in an A/B report.

## Accepted decisions

| ADR | Decision it fixes here |
|-----|------------------------|
| [ADR-0057](../decisions/ADR-0057-refuse-parity-verdict-over-a-placed-note.md) | A lowering that places a note is `UnsupportedScope`, and no parity verdict may read such an outcome, until Phase 6's composition law |
| [ADR-0056](../decisions/ADR-0056-v1-to-v2-consumer-boundary.md) | The lowerer lives in `pertylizer` behind a non-default feature; V2 is reached only through it |
| [ADR-0025](../decisions/ADR-0025-tuning-representation-and-ownership.md) | A note's magnitudes are a validated key identity and a velocity, so the lowerer sends a saved note's own values rather than substitutes |

## Invariants

1. **LOWER-INV-001** — Every lowering produces a **typed** diagnostic for anything it
   refuses or cannot represent. A diagnostic names its **subject** — the project object, as
   a closed enum — and its **reason**, also a closed enum. Free text is not a diagnostic.
   A refusal carries `Severity::Refused` and produces no plan; something unrepresented
   carries `Severity::Unrepresented` and lowering continues.

2. **LOWER-INV-002** — An outcome's fidelity is **derived** from its diagnostics and is
   never set independently. It is `Faithful` exactly when the diagnostic set is empty, and
   `UnsupportedScope` otherwise. An outcome that holds a diagnostic and claims `Faithful`
   must not be constructible.

3. **LOWER-INV-003** — **No parity verdict may read an outcome that is not `Faithful`.**
   An outcome is `Faithful` only when its diagnostic set is empty (`LOWER-INV-002`), so a
   verdict waits for the **last** unrepresented capability, not for any one of them: a
   lowering that names Phase 8's amplifier pan stage, or the terminating node's stages, or
   two or more notes through one island, is as ineligible as one that named a velocity law.
   The render itself is unaffected by any `Unrepresented` diagnostic and still happens.

   **History.** Until `P06-S004` this invariant carried a velocity clause: every lowering
   that placed a note raised one `Unrepresented` diagnostic for V1's two velocity
   sensitivities, which V2 applied as one scale. ADR-0059 built the composition — the
   envelope's `vel_sens` and the instrument's `velocity_amp_sensitivity` each lower to their
   own destination, `SOUND-INV-021` states the formulas — and the clause is discharged: a
   lowering that places a note carries **no** diagnostic about velocity, and
   `P04-R001` is closed. What remains named on a placed note today is Phase 8's, and the
   general rule above is what the clause always reduced to.

   **A lowering that refuses earlier is covered by the same sentence.** A refused graph, an
   unreadable arrangement, an overlap or a note expression stops before a performance
   exists; the outcome is `UnsupportedScope` through that refusal's own diagnostic.

   This invariant scopes the prohibition to **lowered outcomes**. A controlled V1/V2
   comparison that does not go through the lowerer — an evidence harness building its own
   fixture patches, such as EVD-0013's — is outside it and remains valid.

4. **LOWER-INV-004** — Compatibility knowledge lives in the lowerer and nowhere else. Every
   V1/V2 asymmetry the supported subset can reach is represented or refused here, and V1's
   own defaults and clamps are **read from V1's descriptors** rather than transcribed. The
   V2 render plan carries none of it and must not depend on the lowerer.

   **Every saved field has a stated disposition, and something mechanical asks for it.** Two
   mechanisms, because the types live in two crates. `InstrumentState`, `SequencerTrack` and
   `GlobalProjectState` are destructured **exhaustively, without `..`**, so a new saved field
   is a compile error at the disposition site, and since `P08-S003` so are `Patch` and
   `PatchSettings` in `render::patch_dispositions`, where `effect_chain_order` is the one
   setting the arrangement render reads and `master_volume` and `octave_offset` are
   measured inert offline; `TrackMode` is matched exhaustively for the same
   reason. `Song` and `Pattern` belong to `synth_sequencer` and expose their contents through
   accessors, so they cannot be destructured from here; `Note` and `PatternPlacement` could be,
   and are pinned the same way so that the four persisted lists sit in one test with a
   disposition per field. The lists are taken from each type's JSON schema — not from a
   serialized default, because `skip_serializing_if` hides an empty collection and an empty
   collection is the shape a new field arrives in. Either way a new field fails, with the
   disposition question attached. Neither reaches the types those fields *hold* — a field
   added to `TempoChange`, `AutomationLane`, `TrackSend`, `ModGraph` or `NoteGraph` changes no
   pinned list — so a third mechanism closes the set: every persisted name under
   `ProjectFile`, nested or not, is registered in `lowering/persisted_fields.txt` from the live
   project schema, walked into every `properties` object *and its values* so a struct
   variant's fields inside an enum are reached, and `every_persisted_project_name_is_registered`
   fails with the added and removed names. The register carries no disposition of its own;
   its comments say where each type is read, which is where the disposition is written. Three
   independent reads shaped this: one found `Song` and `Note` under neither mechanism, the
   next found the sentence claiming closure while the nested types were still unpinned, and
   the third found the walk stopping at property keys, which missed
   `AutomationTarget::Instrument`'s fields.

   Each field is represented, refused, reported, or recorded as never reaching audio *with its
   reason*. The pin is not decoration: it is what found `Pattern::processors`, a note-processor
   rack that expands the notes a pattern plays exactly as a per-note ornament does, and which
   the per-note refusal could not see because the rack lives on the pattern.

   **Which fields reach V1's audio is measured, not read off the engine.**
   `crates/pertylizer/tests/offline_instrument_settings.rs` changes one saved field at a time
   and asserts the rendered bytes change; the dispositions cite it. A field it measures as
   audible and this lowerer says nothing about is exactly the silent difference this invariant
   forbids.

   **The rule for choosing a disposition.** Anything that changes *which notes sound* is
   **refused** — the key range, the transpose, a voice-allocation setting, a sidechain source,
   a Mod Grid instance on a shape V2 does not carry, a Note Grid graph, a pattern's
   note-processor rack, a placed pattern's automation on a lane class V2 does not carry, a
   placement length override, a master chain, an instrument two tracks play at differing
   track controls or with differing sends. Anything that only scales or places the sound and V2 has no
   stage for is **reported**: the project's glide, oversampling. Anything V2 carries is
   **represented** and not marked: since `P07-S002b`, an instrument lane on the filter's
   cutoff or resonance or the envelope's attack, decay, sustain or release lowers to override
   writes; since `P07-S003`, a Mod Matrix slot or a Mod Grid target carrying an LFO into the
   filter's cutoff or an oscillator's pitch lowers to a modulation edge, marked for its
   timing; since `P08-S001` the instrument's volume, pan and mute lower onto its mix
   channel; since `P08-S002` the track's fader, pan and audibility lower onto a balance stage,
   the master volume onto the master trim, and the instrument's volume and pan lanes, the
   track's fader, pan and mute lanes and the master volume lane to override writes; since
   `P08-S003` the patch's **insert chain** lowers — V1's distortion in its soft-clip mode
   and V1's delay in its mono mode, in `effect_chain_order`'s order, as instrument-scope
   nodes between the balance and the channel, where V1 runs its chain on the voice sum —
   with every effect module named in the order exactly once by parsed identity: an
   omitted, unknown, repeated or non-effect entry is **refused** naming the problem
   (`CORPUS-0005-C1`; V1 appends an omitted module, which ADR-0021 part 2 forbids), a
   distortion mode other than soft clip, a delay mode other than mono and a tempo-synced
   time are refused naming the parameter, and every other effect type stays an unsupported
   type; since `P08-S004` a track's **sends** into the song's returns and the **returns**
   themselves lower onto V2's bus graph, below. Instrument **oversampling** stays reported,
   under a Phase 8 label since `P08-S003` decided against a rate island in this phase's
   slices: no corpus case oversamples, and V1's island is the voice sum alone, so the chain,
   the channel and the master are outside it either way. A
   placement's gain is **inert**: persisted, settable, and read by nothing that renders,
   measured by `placement_gain_is_inert_in_v1` in `offline_instrument_settings`, so it lowers
   to nothing and is not a mark.

   **A whole project lowers into one plan** (`P08-S002`). Every instrument the project holds
   lowers its voice patch into the one voice scope, where each is its own **island** —
   `SOUND-INV-021` binds a note to its island — and its stages follow in V1's order: the
   velocity scaler and the playing track's balance per voice, then on the voice sum the mix
   channel and, under the parity policy, V1's channel-stage soft clipper; every channel
   feeds one master sum, the master volume as a trim held to V1's own `0..=2` and refused
   outside it, V1's output clamp under the parity policy, and the plan's one output. The
   saved output module lowers to the cable that reaches it and to no node of its own. The
   plan declares one compiled producer of the project's peak simultaneous notes, floored at
   one and counting a note ending where another begins as overlapping, so the voice scope is
   instantiated once per note held at once across the project and a note lands on any free
   instance. Since `P08-S003` two notes of **different** keys held at once on one instrument
   lower to two instances of its island, which is what the corpus's dyad through a shared
   insert chain measures, up to the instrument's voice count — V1's default, since any other
   is refused — beyond which V1 steals a sounding voice and the lowering refuses by name, a
   tie counted as held; two of **one** key stay refused by name, because a compiled release
   names the newest open note with its key and the first note's off edge would release the
   second. The mark that remains is the release: V1 keeps a voice per note through its
   release while V2 frees a note's index at its off edge, so a later note may retrigger an
   instance whose release still rings. A track soloed anywhere silences every unsoloed track's notes before they are
   lowered, as before; an instrument soloed anywhere starts every unsoloed instrument's
   channel muted, as V1's mix stage skips it — the lowering specification's solo-elsewhere
   question is closed by the whole project being the input. The tracks that play an
   instrument are those a span comes from: none leaves it no balance stage, one owns the
   stage, several with equal static controls share one stage and a track lane on any of them
   is refused until ADR-0034, and several with differing controls — V1's per-voice gain — are
   refused by name until ADR-0034. A track lane on a track that plays nothing is V1's write to
   a control slot no voice reads and lowers to nothing. V1 clears its track control map and
   its module overrides where its transport stops, so those lanes restore the authored value
   at the song's end; the instrument's own fader and pan and the master volume are set by
   their lanes and never restored, as V1 sets them. Events at one sample keep their emission
   order — a note's on edge before its own release, which a note whose edges round to one
   sample depends on — and the declared notes are counted over the same rounded sample
   positions the events are emitted at, a note ending at the sample another begins at counted
   as overlapping, so a note-on presented before the release that would have freed an index
   still finds one. The one-instrument lowering refuses a track lane on a playing track and
   an instrument volume or pan lane by name where its plan holds no balance stage or channel,
   rather than dropping either as inert. An instrument identity past the address space's
   instrument field is refused by name rather than folded into another's; every node address
   is a function of the instrument's identity and the module's, and the nodes the lowerer
   inserts sit in a per-instrument range no saved module reaches. Three or more instruments
   saved out of identity order are a marked difference — V1 sums the master in list order and
   V2 in identity order (`SOUND-INV-008`), and a float sum of three terms depends on its
   order. The caller selects the output policy: **parity** places V1's two saturation stages,
   **headroom** places neither and preserves the float sum. Not built here, under the phase's
   YAGNI: a per-instrument voice group of its own — every island is instantiated once per
   simultaneous note across the project, which is bounded and correct and costs idle
   instances on a project whose instruments seldom sound together.

   **Sends and returns lower onto the bus graph** (`P08-S004`, `SOUND-INV-034`), read at
   V1's own boundary. Every return the song declares lowers, fed or not, as V1 creates every
   one: an entry sum, its effect chain from `return_bus_effects` in the order the project
   stores it through the same insert lowering as an instrument's — the distortion and the
   delay carried, every other type refused by type on the return's own effect — a strip with
   V1's return fader, pan and mute, and under the parity policy V1's return clipper; its
   output enters the master unless another return is soloed, since V1's return solo gates
   the master sum alone, and its bus-to-bus sends read that output — clipped, as V1 taps it
   — and enter their targets' entries. Every node of a return sits in the return's own bus
   scope, addressed under one shared bus slot above every instrument's and below the
   master's; a return identity or an effect instance past that slot's eight-bit fields is
   refused by name. An instrument's sends are the sends of the tracks **assigned** to it,
   because V1 keys its send lists by instrument and rebuilds them from every assigned track
   in list order, the last one's list replacing the rest, an empty track included: two
   assigned tracks with differing lists are refused by name until ADR-0034, equal lists are
   one. From that list V1 resolves the enabled sends whose target exists and keeps the first
   sixteen; the count is checked against the profile's `max_sends_per_channel` **before** the
   zero-level sends are dropped, since a resolved send at zero occupies one of V1's slots,
   and a seventeenth is refused naming `LIMIT-0024` where V1 dropped it. A zero-level send
   and a disabled one lower to nothing, V1's documented no-ops; a send into a return the
   song does not declare, or a return's send into itself, is what V1 skips silently and is
   refused by name. A pre-fader send is a `Send` reading what the channel's strip reads; a
   post-fader send is a `PostFaderSend` reading the same signal and carrying the strip's
   fader and pan beside its level, so an instrument volume or pan lane fans out to every
   post-fader send's copy; both start muted where the instrument is muted or soloed out,
   since V1 taps nothing from a channel that is not audible. A track mute lane zeroes the
   voices before the inserts, as V1's does, so an insert delay's tail still feeds the send
   past the mute. The summation-order mark is generalised: V1 sums the master's returns in
   its per-block Kahn order and each return's input in instrument list order then Kahn
   order, V2 every sum in ascending source identity, so a sum of three or more terms whose
   two orders differ is marked on the master or on the return — the lowerer walks V1's
   Kahn order over the enabled resolved bus sends to know. The corpus's send case
   (`CORPUS-0004`) lowers its send and its return and no longer names either; what stays
   refused is its reverb by type and its master compressor, owed to `P08-S007`.

   **The song's end is V1's.** `Song::calculate_length` — the later of the last placement's
   end and the last section's — is where V1's sequencer auto-stops and releases every note it
   holds, so a note held past it is released there in the lowering too, and the render extends
   to it even when the last release comes earlier: a trailing rest, or a section drawn past the
   last placement, is silence V1 renders. A placement whose end would overflow that function's
   unchecked addition is refused by name first. An independent read of the squash found the
   authored release used and the render stopping at the last release.

   **Topology is keyed by resolved identity, not spelling.** `ModuleId` parses its instance as
   a number, so `amp-01` and `amp-1` are one module to V1; the tables that decide whether an
   amplifier's control is patched, whether a cable is a verbatim repeat, and whether a port
   receives fan-in compare parsed identities and keep the spelling only for the diagnostic. A
   spelling that does not parse is refused by name where it appears. The same read found the
   tables keyed by text, which called a respelled control unpatched.

   **An absent choice is the descriptor's declared default.** V1 creates a module from its
   descriptor and applies the saved parameters over it, so a saved map that omits `waveform`
   or a filter `type` means the descriptor's default there; the lowerer reads
   `range.default` as the index into the descriptor's `choices`, the way `gen_schemas`
   reads it, rather than naming the default in a literal that would outlive V1's. The same
   read found two literals.

   **A tempo ramp is a marked difference, not a translation.** The two `TempoChange` types
   share their fields, but V1 ramps the tempo number linearly in tick space and V2 ramps the
   beat's period (`SOUND-INV-019`), so every event after a ramp that has a later change to
   ramp toward lands elsewhere. ADR-0049 accepts that as an intentional semantic change that
   must map to a comparison category, so such a lowering is `UnsupportedScope` with a
   diagnostic naming it; a ramp with nothing after it, and a step, are exact.

   **The declared event peak is counted as admission counts it.** Admission slides a `Q`-frame
   window over the plan's edges — the worst case over every anchor phase — so the lowerer
   declares the same figure, not a count per absolute quantum, which admits a plan whose
   stream is refused after a seek. The review of the squash found the buckets and the
   forwarded ramp flag.

   **A value V1 normalizes is read through V1's own boundary rather than compared raw**, and
   through the *same function* V1's loader calls where one exists. A saved key range of
   `(127, 0)` is the full keyboard, because `KeyRange::new` swaps reversed endpoints; a saved
   oversampling of `3` is `X1`, decided by calling the loader's own decoder rather than a second
   copy of its `match`; and an instrument transpose of `0.4` moves no note, because
   `MidiNote::transpose` rounds. Each of those is neutral here exactly as it is to V1.

   **Audibility is checked where the state acts, not where the notes are — and only state V1
   acts on is refused.** V1 runs a pattern's automation whether or not that track's notes are
   audible, so automation is inspected over every placement before any note-level filtering;
   but a lane with no points emits nothing there, so only a lane holding a point is automation.
   A Mod Grid graph runs when V1's own builder makes an instance of it, which a graph with no
   routing sink or a track-scoped graph assigned to no track does not, so the lowerer asks that
   builder rather than the pool. A note-processor rack or a Note Grid binding acts where V1
   expands a pattern — on a placement that passed the instrument, mute and solo filters — so
   it is refused there and not on a pattern the arrangement never plays; a binding is resolved
   through the pool as V1 resolves it, and a dangling one is the pass-through it is in V1. An
   independent read found each of the three refusing a project V1 plays unchanged.

   **A placed pattern's instrument automation is lowered as V1 runs it** (`P07-S002b`). Each
   lane on the filter's cutoff or resonance or the envelope's attack, decay, sustain or
   release, naming this instrument, is walked over every tick its placement is active — on a
   muted track, and on a track routed to another instrument, since V1 runs it there too — and
   every value V1's sequencer would emit becomes one override write: the tick is
   `pattern_tick_at`'s, the curve is `value_at`'s, the threshold is the sequencer's own
   constant, the range is the descriptor's `denormalize`, and the module is the lowest
   identity of its type, which is what V1's `BTreeMap` hands `apply_normalized_override`. A
   lane naming another instrument is that instrument's and is skipped as its notes are; a
   lane on a module the patch lacks is V1's no-op and lowers to nothing. V1 clears its
   transient overrides where its transport stops, which the offline renderers do at the song's
   end before a tail, so one write of the prepared base per touched slot lands there. Two
   lanes on one target whose active tick ranges intersect are two absolute writers at one
   sample and are refused naming both patterns and the target, whatever their values; so are
   two lanes whose emissions round to one frame, since disjoint ticks are not disjoint samples
   at a low rate and a high tempo. Inert lanes conflict over nothing. The walk is bounded in
   ticks and refused by name past the bound, because no frame bound bounds a tick count. The
   writes count toward the declared event peak beside the note edges, from the same walk.

   **`P07-S004` carries V1's six Mod Matrix macros on the same edges.** Velocity, note
   number and polyphonic aftertouch lower to voice-scoped note sources; channel aftertouch,
   mod wheel and pitch bend lower to instrument-scoped controller sources. Each used macro
   creates one source at its reserved address beside the voice-output scaler, outside saved
   module and Mod Grid address ranges. Two slots reading one macro share that source. The
   target scale and dangling-endpoint rules below apply unchanged. V1 reads current macro
   state once per host block; V2 samples source changes at quantum boundaries, so each macro
   edge reports that timing as unrepresented. An absent live controller remains zero; no
   controller performance is invented for a saved project. The generic Mod Grid source
   classes remain refused for their owners.

   **A Mod Matrix slot and a Mod Grid target lower to modulation edges** (`P07-S003`). V1
   hands `source × amount` to the destination module's `set_mod_offset`, which scales it into
   its own unit — the filter's cutoff by ±48 semitones, an oscillator's `frequency` by an
   octave, its `pitch` by one semitone, its `detune` by one; ADR-0007 clause 3 makes that
   per-target scale the edge's amount, so a slot at `0.7` into a cutoff is an edge of
   `0.7 × 48` semitones, read from V1's own constants rather than transcribed. A row exists
   only where V1's arithmetic for the key **is** the law V2 declares for the parameter —
   semitone-additive on the cutoff and the three pitch keys, which V1 accumulates unclamped —
   and every other destination is refused naming the law V1 applies that V2 does not: the
   resonance's normalized offset mapped into a quality factor, the oscillator's linear level
   under V2's decibel law, an LFO's rate in hertz under V2's semitone law, an LFO's depth
   whose offset V1 clamps after each contribution where V2 clamps the sum, an envelope time
   through its descriptor's normalized curve. The source is the LFO — V1's module lowers to
   V2's `Lfo` kind in the voice scope, waveform, rate and depth through the descriptor and
   the phase through V1's own typed conversion, which **wraps** a saved `1.25` to a quarter
   period where the descriptor's clamp would make it the start — and an envelope, a
   audio output and a module parameter read as a source are each refused
   by name for their owners; so is a scripted slot, a random shape, a tempo-synced LFO and a
   cable out of an LFO. No lowered target is an LFO's parameter, so no modulation cycle can
   close today; the slice that gives the LFO a lowered target owes the refusal that names the
   slot. The matrix V1 walks is the lowest identity of its type, as `Voice::from_graph` asks
   its map; a second matrix is stored and never read. The saved strings become addresses
   through the two parsers V1's loader calls, legacy spellings included, and the enabled flag
   through the descriptor's own conversion. A disabled slot, a slot with no destination or no
   source and a spelling neither parser accepts are V1's own skips and lower to nothing; so
   does an address naming a module the patch lacks, **whatever kind or law it would otherwise
   name**, because V1 reads zero from a dangling source and applies nothing to a dangling
   destination — both ends are checked before either is classified. **Each edge is a marked
   difference**: V1 reads a source once per host block — the voice's matrix reads the
   previous block's first sample, the grid processes its instance and reads the current
   block's last — and V2 composes the current quantum's first frame ahead of the quantum's
   writes; the corpus records that as an intentional correction (`CORPUS-0003-C1`), so the
   lowering is `UnsupportedScope` with a diagnostic naming the slot, as a tempo ramp is under
   ADR-0049's rule. A **Mod Grid**
   instance is what V1's builder returns, read through it: a global instance's hosted LFOs —
   read back from the modules V1 built, so V1's clamps and wraps are already applied — become
   global-scope nodes, and each module-backed target on this instrument, at the address the
   builder interned, becomes an edge through the same table; the target is settled before its
   source is read, so a target on another instrument is that instrument's whatever feeds it,
   as its notes are, a target on a module the patch lacks is V1's no-op, and a target with no
   cable, or with a cable from a port the hosted module lacks, is V1's `continue` or its
   zero. A track-scoped instance, a track, master or channel-level
   target, a macro, transport, MIDI CC or audio-tap source, an injection into a hosted module,
   a cable into one, and a hosted module other than an LFO are refused by name. The corpus's
   Mod Matrix case lowers, so three saved projects lower where `P04-R002` recorded two. The
   corpus's insert-chain case lowers since `P08-S003`, and since EVD-0021 reselected the
   session share under ADR-0054 (`P08-S004`) it renders under the engine's default profile
   too — its catch-up addresses number 29, which the first provisional share of 24 refused
   by name and the reselected share of 128 admits — so the default-profile survey counts
   four.

   **Where that stops, stated so it is a rule rather than a gap.** A stage is refused when V1
   installs it on a placement it walks and it has something to act with: a lane with a point
   on a class V2 does not carry, a Mod Grid instance V1's own builder returns on a shape V2
   does not carry, a rack with a processor, a bound graph with a node. What V1 short-circuits
   *before* running is neutral — a lane with no points, a zero-length pattern or a zero-length
   placement override that `pattern_tick_at` resolves no tick into, a graph with no nodes
   whose expansion is its seeded source, a builder that returns no instance, a Mod Matrix slot
   V1 skips before reading it, a Mod Grid target with no cable. What an installed stage then
   **computes** is not evaluated: a rack over a pattern with no note, a node off the graph's
   spine. These are refused exactly as a master effect at neutral settings is refused rather
   than measured, because that class has no floor — every installed stage has a setting at
   which it does nothing — and the contract would otherwise promise a neutrality analysis of
   V1 it does not perform. A second independent read asked for three of those refinements;
   this paragraph is the answer. A zero modulation amount is not in either list: V1
   evaluates the slot and adds zero, so it is an edge of zero depth.

   **And the other direction is the one that matters more.** V1 evaluates two per-note things
   on every active tick regardless of the note's own start: a note-scope graph, which it seeds
   for every note, and an ornament, whose lead-in hits land before the note's onset. A
   source-independent generator or a lead-in figure on a note past the pattern's end therefore
   still sounds inside it, so both checks run before the hidden-note skip; an expression acts
   only when the note itself plays and stays after it. Two reads found one each on the wrong
   side, which was a silent difference rather than a false refusal. The rule's precedence is
   V1's own too: a resolved Note Grid binding is the arm V1 takes, so a rack under a node-less
   graph never runs and is not refused; a graph with a node is.

5. **LOWER-INV-005** — Lowering resolves a project's string and positional identities into
   stable typed identities. No file loading reaches V2, and assets arrive already prepared.
   Project save and load are unchanged: the lowerer is a consumer only.

## Types and ownership

The lowerer owns its diagnostics and its outcome; the caller owns what it does with them.
No V2 type carries a lowering concept.

```rust,ignore
pub enum Severity { Refused, Unrepresented }
pub enum ProjectSubject { /* closed: project, track, instrument, module, connection, ... */ }
pub enum LoweringReason { /* closed, and includes: */ OwnedByLaterPhase { capability: &'static str, owner: &'static str } }

pub struct LoweringDiagnostic { /* private fields; built through `refused` or `unrepresented` */ }

pub enum Fidelity { Faithful, UnsupportedScope }
impl Fidelity {
    pub fn of(diagnostics: &[LoweringDiagnostic]) -> Self;   // LOWER-INV-002: derived, not set
    pub const fn admits_parity_comparison(self) -> bool;     // LOWER-INV-003: false unless Faithful
}
```

## Lifecycle and timing

Lowering is pure and happens entirely off the audio thread. It reads an already-loaded
project value. It has two halves with different positions against compilation, and the
order is forced rather than chosen: **graph** lowering produces the `ProjectGraphIr` a plan
is compiled from, so it precedes compilation; **performance** lowering produces the events,
and an event names a note slot only a `CompiledPlan` can resolve, so it follows compilation
and reads the plan. Neither half mutates what it produced, and no step of either runs while
a render is in progress. An earlier revision of this paragraph placed all lowering before
compilation, which the event half cannot satisfy; an independent read found the claim.

## Failure and diagnostics

A refusal yields no plan and at least one `Refused` diagnostic naming the subject and
reason. An unrepresented capability yields a plan, at least one `Unrepresented` diagnostic,
and `UnsupportedScope`. A caller that only wants to know whether it may compare asks
`Fidelity::admits_parity_comparison`; a caller that wants to tell the user what happened
reads the diagnostics, each of which names its own subject.

**What the fail-closed boundary is, stated exactly.** It is the derivation in
`LOWER-INV-002` plus `LOWER-INV-003`'s prohibition, and there is no comparator in the tree
for either to gate. A lowered outcome's samples and diagnostics are readable, so a future
caller could compare them without asking for the verdict; what prevents that today is that
no such caller exists and ADR-0057 clause 5 refuses building one. The first consumer that
needs a parity verdict brings the encapsulation with it. `P04-R001`, which it once had to
inherit first, is closed by ADR-0059.

## Real-time and resource constraints

`N/A` for the audio thread: lowering never runs on it, allocates freely, and produces a
value the engine admits afterwards. The plan it produces is admitted against `HostProfile`
by the Sound Core render contract's own rules, which this specification does not restate.

## Conformance tests

| Invariant | Named test or evidence |
|-----------|------------------------|
| LOWER-INV-001 | `pertylizer`'s lowering tests: refusals naming an unsupported module type, an unsupported waveform, an unresolved endpoint, an unknown port, a domain mismatch, fan-in, a second output, a missing output, a note expression, an overlap, a muted instrument, a note graph, and a persisted transpose refused **by value** before the arithmetic that would panic on it |
| LOWER-INV-002 | `Fidelity::of` takes the diagnostics and returns the verdict, so the two cannot disagree; `a_placed_note_names_no_velocity_gap_and_still_refuses_a_parity_verdict` asserts on a real lowering that what remains named is Phase 8's alone, that the verdict is therefore `UnsupportedScope`, and that it refuses a parity comparison |
| LOWER-INV-003 | `Fidelity::admits_parity_comparison` is `Faithful` alone by construction. The discharged velocity clause: `a_placed_note_names_no_velocity_gap_and_still_refuses_a_parity_verdict` asserts that no diagnostic names the velocity composition over one note and over four, that every remaining diagnostic is Phase 8's, and that the outcome still refuses a parity verdict through them; `v1s_two_sensitivities_compose_to_the_velocity_squared` asserts the lowered product at V1's defaults as a peak ratio of a quarter, which a single scale fails at a half. Its scoping is checked by EVD-0013's harness, which compiles a V2 graph directly and never calls the lowerer |
| LOWER-INV-004 | `crate_boundary`, measured with `cargo tree --edges normal --invert` under default features; `synth_engine_v2` has no dependency on the lowering module. For the saved-state half: `every_persisted_song_field_has_a_disposition` pins the persisted field lists of `Song`, `Pattern`, `Note`, `PatternPlacement` and `SequencerTrack` with a disposition per field; `every_persisted_project_name_is_registered` pins every persisted name under `ProjectFile`, nested types included, against `lowering/persisted_fields.txt`; `every_audible_instrument_setting_is_dispositioned` walks the fields `offline_instrument_settings` measures as audible and asserts each is refused or reported, with a neutral instrument as its control; `song_level_state_is_refused_rather_than_ignored` covers the Mod Grid graph — routed and track-scoped and assigned refused as such, routed into a track's volume and global refused for the target, empty or unassigned rendering — the note-processor rack on a placed pattern against one unplaced, placed only on a muted track or zero-length, or zero-length under a length override, and the rule's boundary pinned as a rack on an audible pattern with no note, the Note Grid pool against a node-less binding, a rack shadowed by a node-less binding, a binding with a node, a dangling binding and a note-scope binding on a **hidden** note, the length override and the rounded-away transpose; `a_note_expression_is_refused_rather_than_played_as_authored`, whose second half puts a lead-in ornament on a hidden note; `the_render_is_bounded_by_the_song_end_as_v1_bounds_it`, a release past the song's end and a section past the last placement; `a_cable_spelled_with_a_leading_zero_resolves_as_v1_does`; `an_absent_choice_lowers_as_the_descriptor_declares`, which reads the declared default itself and asserts sample equality against it; `the_declared_event_peak_slides_a_window_as_admission_does`, two edges across an absolute quantum boundary; `a_tempo_ramp_toward_a_later_change_is_marked_unrepresented`, against a step and a trailing ramp as controls; `a_zero_length_override_is_as_inactive_as_a_zero_length_pattern`; `instrument_note_input_is_refused_rather_than_ignored`; `track_mixer_state_is_reported_rather_than_ignored`; `pattern_automation_is_refused_rather_than_flattened`, whose first half asserts a lane with no points renders, whose middle places the automation on a **muted** track so moving the check back behind the note filter fails, and whose last half places a pointed lane on a zero-length pattern; and `project_global_state_is_read_rather_than_ignored`. **`P07-S002b`'s lowered lanes** are held by the tests `SOUND-INV-023`'s row names in the Sound Core render contract, over the same fixtures; the refused classes by `the_lane_classes_v2_does_not_carry_are_refused_by_name`, one per class with its owner. **`P07-S003`'s modulation edges** are held by `the_corpus_mod_matrix_slot_lowers_to_one_edge_at_v1s_scale` (`CORPUS-0003` from its pinned bytes: one edge of `0.7 × 48` semitones from a voice-scope triangle LFO at 2 Hz into the filter's cutoff, the matrix no node, the timing the one diagnostic about either, the render differing from the slot disabled, and a zero amount rendering exactly as disabled while lowering to one edge of zero depth), `a_mod_matrix_slot_lowers_v1s_legacy_spellings_and_each_pitch_key_at_its_scale` (`lfo1` and `osc1_pitch` through V1's parsers, the three pitch keys at V1's three scales onto one frequency control, four edges compiling), `an_inert_mod_matrix_slot_lowers_to_no_edge_and_no_diagnostic` (disabled, no source, no destination, dangling either way — including a dangling envelope source, a dangling resonance destination and a macro into a dangling module — unparsable either way, an unwritten slot, and a second matrix in both directions), `the_mod_matrix_routes_v2_does_not_carry_are_refused_by_name` (the envelope in both spellings, an audio output, a parameter source, ten destinations naming their V1 law, a scripted slot, four random-shape spellings, tempo sync, and a cable out of an LFO into the amplifier), `an_lfo_lowers_with_v1s_defaults_clamps_and_wrap` (absent keys, a rate past `LFO_RANGE`, a depth past one, a phase of one period, of one and a quarter, and of minus a quarter), `a_global_mod_grid_lfo_into_a_module_target_lowers_to_a_global_node_and_edges` (a hosted LFO read back from V1's module — waveform by index, phase wrapped — as one global node feeding a module target and a module-backed instrument target at V1's scale, and the render carrying it), `the_mod_grid_shapes_v2_does_not_carry_are_refused_by_name` (track scope, four cheap sources, four targets, a hosted envelope, a module-to-module cable, an injection, a random shape; another instrument's target from an LFO and from a macro, a target on a module the patch lacks and a cable from a port the LFO lacks, each under a carried and an uncarried law, and a cable-less target inert) and `a_mod_grid_node_address_cannot_meet_a_saved_modules_or_the_scalers`. Mutation-verified in seventeen further directions: the cutoff scale dropped, the grid scale dropped, a disabled slot walked, the highest matrix applied, a zero amount skipped, a dangling target's law refused before its existence, a dangling grid target lowered as present, the timing report dropped, the LFO in the global scope, a grid node in the voice scope, a cable out of an LFO admitted, the phase clamped before the wrap, the phase not wrapped, any output admitted as a source, tempo sync admitted, a track-scoped instance lowered, and another instrument's target lowered. Mutation-verified in thirty-four directions: dropping either note-input refusal, the oversampling report, the sidechain refusal, the automation refusal, the Mod Grid refusal, the note-processor refusal, the pattern Note Grid refusal, the note-scope refusal, the length-override refusal, the fader report; reporting the fader per placement rather than per track; comparing the key range as a raw tuple; comparing the transpose without V1's rounding; reading the lane list's length instead of its points; reading the Mod Grid pool instead of V1's builder; scanning every pattern for a rack instead of the placements V1 plays; reading a Note Grid binding as a bare `Option` instead of through the pool; moving the note-scope check or the ornament check behind the hidden-note skip; refusing a node-less bound graph, or the rack beneath one; refusing a zero-length pattern's lanes or its rack, or its rack under a length override; stopping the register's schema walk at property keys; keying the amplifier's control check by spelling; naming a waveform default in a literal; leaving a release unclipped past the song's end; stopping the render at the last release; bucketing the event peak by absolute quantum; dropping the ramp diagnostic; and refusing a zero-length override as an override. **Both mechanisms are mutation-verified**: adding a field to `InstrumentState` produces `E0027` at the disposition site, and the persisted pin failed on first run — which is how `Pattern::processors` was found. `each_mod_matrix_macro_lowers_once_at_v1s_target_scale_and_in_its_scope` holds all six `P07-S004` macros to one source per identity, two edges at the V1 scales, their voice or instrument scope and the timing diagnostic. **`P08-S002`'s whole-project lowering** is held by `lowering::tests::phase8`: `two_instruments_on_one_patch_are_two_channels_summed_exactly` (the first exit bullet — two instruments on one patch definition at two faders, two notes at once, render the sum of each alone bit for bit under the headroom policy), `an_instrument_soloed_elsewhere_silences_this_one` (the soloed render equals the soloed instrument alone in value, the unsoloed one's notes still lowered, and nothing named), `the_parity_policy_clamps_at_full_scale_and_headroom_preserves_the_sum` (both faders at V1's maximum: the headroom render peaks past full scale, the parity render is held at exactly one, and frames between the knee and full scale are lower under parity), `the_chain_is_v1s_order_into_one_master` (the inserted stages' order, scopes and kinds, one output at the master, the saved output module lowered to no node, one channel per instrument), `a_shared_instrument_with_differing_track_controls_is_refused_by_name` over the corpus's `shared-instrument-tracks` project, and the same tracks at equal faders lowering to one stage, `the_declared_notes_are_the_projects_peak_across_instruments` (overlap two, separated one, none one, back-to-back two, and that project rendering), `instruments_out_of_identity_order_are_marked_at_three`, `an_instrument_beyond_the_address_space_is_refused_by_name`, and `a_velocity_macro_lowers_beside_the_channel` (the address the `P08-S001` channel shared with the velocity macro). In `tests.rs`: `the_lane_classes_v2_does_not_carry_are_refused_by_name` holds the instrument's volume and pan lanes, the host track's fader, pan and mute lanes and the master volume lane to one write each and a restore only for the track's, a track lane on a silent track to nothing, and a track lane on a shared instrument to the ADR-0034 refusal; `track_mixer_state_is_lowered_rather_than_reported` and `project_global_state_is_read_rather_than_ignored` hold the track fader and pan and the master volume to the render — half the master is half every sample, bit for bit — with a master past V1's range refused. **`P08-S004`'s sends and returns** are held by `lowering::tests::buses`: `the_corpus_send_case_no_longer_names_its_send` (`CORPUS-0004`: the compressor the one refusal, and with it cleared the reverb by type on the return's effect), `a_post_fader_send_into_a_delayed_return_renders_wet_beside_dry` (a muted return and a send at zero are the same render, the wet path arrives after the delay line), `the_bus_graph_is_lowered_in_v1s_shape_and_scopes` (the send in the channel's scope reading what the strip reads and entering the entry; the return's entry, delay, strip and clipper in its scope into the master; nothing else into the master), `a_pre_fader_send_reads_before_the_fader_and_a_muted_instrument_sends_nothing` (the render at any fader is the dry path plus the wet path at a fader of zero, bit for bit; a muted instrument renders silence), `sends_are_refused_by_name_where_v1_dropped_them` (sixteen zero-level sends and an audible seventeenth refused naming `LIMIT-0024`, sixteen lowering with one send in the plan; an undeclared target; a self-send; a return past the address space), `an_instrument_assigned_to_two_tracks_with_differing_sends_is_refused_by_name` (and equal lists lowering as one), `a_return_soloed_elsewhere_drops_the_master_cable_and_keeps_the_bus_send`, `a_return_with_an_uncarried_effect_is_refused_by_type_and_an_empty_one_lowers` (an unfed return adds exactly nothing), `two_instruments_on_one_patch_keep_independent_sends`, `a_return_order_v1_walks_differently_is_marked_at_three` (on the master, and on a return fed by three instruments out of identity order), `an_instrument_volume_lane_reaches_the_post_fader_send` (a lane holding one value renders as the value saved, bit for bit) and `a_track_mute_lane_leaves_the_insert_delays_tail_feeding_the_send` (with the insert-free control silent). `lowering::tests::evidence` holds EVD-0021's control — a plan with no writable control requests one session event — and its consequence, every measured project below the selected share admitted under the default profile at its measured request. |
| LOWER-INV-005 | `no_file_loading_reaches_v2`, which scans both production trees recursively and is mutation-verified against a nested module, the repository's own `project::load_file`, and V2 reading a file itself; `ResolvedIdentities` for the identity half |

## Unresolved questions

| Question | Blocking? | ADR or task |
|----------|-----------|-------------|
| Where a comparison harness reads a lowered outcome, and what encapsulation it needs so `LOWER-INV-003` cannot be bypassed | Yes for the first comparison consumer | ADR-0028 and Phase 10B for the surface; ADR-0057 clause 5 refuses it meanwhile |
| Which further V1 module types the lowerer supports | No — a per-phase subset choice, refused by name meanwhile | Phase 5 and later |

## P07-S007 Note-processing disposition

Saved Note Grid bindings and note-processor racks remain named refusals under
P07-R001, owned by Phase 10A's canonical note-processing model. SOUND-INV-030
introduces a finite standalone Note YAMS source with explicit quantum context
and cut semantics; this does not establish a faithful lowering of those saved
structures. Existing refusal coverage in `lowering/tests.rs` remains binding.
