# ADR-0026: Minimum SampleMap and SampleZone Model

| Field | Value |
|---|---|
| ID | ADR-0026 |
| Status | Proposed |
| Phase | 6/10A/10D |
| Created | 2026-09-06 |
| Last reviewed | 2026-09-06 |
| Related | ADR-0025, ADR-0059, ADR-0058, `SOUND-INV-016`, `SOUND-INV-021`, `SOUND-INV-025`, `P06-S005`, `EVD-0007`, master plan question 26 |
| Supersedes | — |
| Superseded by | — |

## Durable boundary

Three boundaries. It is a **product** boundary: what a sample zone *is* — which fields
select it and shape its playback — is the vocabulary every later sampler feature (multi-zone
maps, round-robin, slicing, streaming) authors against, and the master plan requires that
"the initial one-sample sampler is represented as one zone rather than a special
incompatible model". It is **delivered behaviour**: V1's sampler has a playback law — how a
key becomes a rate, how it interpolates, how a note ends — and a saved sampler patch
rendered under another law sounds different. And it is a **real-time and contract** boundary:
a prepared sample is the first immutable payload in V2 that a `Copy` prepared record cannot
hold, and a sampler beside an envelope is the first node that must start on a note without
being the node the note plays.

**Why it is ready now.** `P06-S005` is the next slice — "the one-zone sampler on the
prepared map/zone contract" — and cannot build under an undecided zone model:
`PROCESS.md`'s decision-timing rule makes a registered question slice-binding when the next
slice cannot proceed safely without it. Phase 6's exit gate names it: "a native one-zone
sampler uses the prepared map/zone contract without per-note allocation or a special
single-sample voice API".

**Coupled decisions.** [ADR-0025](ADR-0025-tuning-representation-and-ownership.md) fixes that
a note-on names a key identity and that no node converts a key to a frequency on its own;
this record's zone selects by that key and its rate law reads the frequency the scope's
prepared tuning resolves. [ADR-0059](ADR-0059-velocity-composition.md) fixes that a velocity
destination is a kind's own control with its own sensitivity; this record adds V1's sampler
sensitivity as a third such destination and composes nothing new. **Not decided here**: the
persisted `SampleAsset` — its digest, its embedded-or-external policy, its provenance — is
Phase 10A's and 10D's; this record fixes only what a *prepared* sample is and what a zone
names, and states the constraint the persisted form must later meet.

## Decision boundary

It decides the **zone and map types** a V2 sampler consumes, the **one-zone subset** Phase
6 builds and how the rest is refused, what a **prepared sample reference** is in the IR and
in the plan, how a sampler node **starts and ends on a note** without being playable, and
V1's **playback law** where V2 reproduces it. It does not decide multi-zone selection order,
round-robin, streaming, slicing, time-stretch, recording, the persisted asset form, or the
lowering of a saved sampler module, which waits for the first sampler corpus case.

## Evidence

Read from V1's own code rather than its descriptors, and from the plan documents:

- **V1 has no zone.** Every field the master plan calls a zone's lives on the sample's
  metadata: `SampleMeta` carries `root_note: Option<MidiNote>`, `loop_region`
  (start, exclusive end, and a `crossfade` **that the player never reads**) and `crop`
  (`crates/synth_sampler/src/types.rs:167-181`). No key range, velocity range, per-zone
  gain or selection policy exists anywhere in V1. The sampler module references its audio by
  `SampleId`, a library-assigned `u64` persisted as `{"sample_id": N}`
  (`crates/pertylizer/src/patch.rs:364-375`); audio travels embedded in a bundle keyed by
  that id, with no content digest.
- **V1's rate law is a frequency ratio, not a semitone offset.** `set_voice_pitch` computes
  `base_speed = played_frequency / root_frequency × 2^(fine_tune_cents / 1200)`, refreshed
  every block so glide and bend track (`crates/synth_modules/src/sampler.rs:410-424`), where
  the root frequency is `MidiNote::to_frequency()` — V1's hard-coded twelve-tone
  conversion, which the master plan forbids a V2 node to perform. A separate
  `from_note_offset` helper computing `2^((target − root)/12)` exists and is unused.
- **V1 interpolates linearly**, two taps per channel, wrapping the second tap into the loop
  start at the loop boundary and clamping it at the crop end
  (`crates/synth_sampler/src/playback.rs:283-320`). A stereo sample is rendered interleaved
  and **downmixed to mono** for the module's single output: `(left + right) × 0.5 × level`
  (`sampler.rs:300-306`).
- **V1's note end is a fade inside the player**, not an envelope: `Sustain` mode's
  `note_off` starts a hard-coded linear fade of 512 frames (`playback.rs:110`, "~10 ms at
  48 kHz"); `OneShot` ignores the release and plays the region out; `Loop` loops the region
  until release. Direction is a second axis — `Forward`, `Reverse`, `PingPong`. `start_offset`
  maps into the crop range and is applied only above `0.001`.
- **V1's sampler applies velocity itself**: `vel_gain = (1 − s) + s × v` at note-on
  (`sampler.rs:474-476`) — the same law ADR-0059 gave the voice-output scaler. A V1 sampler
  patch with an envelope and the instrument's amplitude sensitivity therefore applies one
  velocity **three** times at the defaults.
- **V1 allocates a `SamplePlayer` per note-on** (`sampler.rs:443-449`) and receives its
  audio through an optional no-op trait method, `PolyModule::load_sample_data`, which drops
  crop and loop on the way in. Phase 6's exit gate and the master plan's node contract rule
  both out.
- **V2's precedent for shared immutable prepared data is the tuning.** A `PreparedTuning` is
  declared per execution scope in the IR, prepared off-thread with a content digest, held
  **once** in a plan-side table, referenced by a `TuningSlot` index that resolves to one
  array read on the audio thread, deduplicated by comparing the tables themselves rather than
  their digests, and charged once to `prepared_immutable_bytes` plus one slot per reference
  (`crates/synth_engine_v2/src/compile.rs:1696-1717`, `ir.rs:915-956`). `PreparedNode` is
  `Copy`, so PCM cannot go where a prepared oscillator goes.
- **One scope holds one playable node.** `SOUND-INV-021`'s refusal `AmbiguousNoteScope`
  stands after `P06-S001` (`compile.rs:1650-1668`): a note's address is one node, and every
  other node in the scope receives the note through declared destinations. A sampler beside
  the envelope that plays the voice cannot be a second address.
- **No sampler corpus case exists.** `corpus/v2-reference/manifest.json` lists
  `sampler-patch` as the one planned category, owned by Phase 0B's bundle fixtures, because a
  sampler case needs its sample asset to travel with the project. The only saved sampler
  module in the repository (`all-modules-reference.ptz`, `sam-1`) references `sample_id: 0`,
  which is no sample.

## Options

Three questions, each with the shape chosen.

**Q1 — the model and the reference.**

1. **The master plan's split, with the prepared sample a plan-side table.** `SampleZone`
   carries key range, velocity range, root key, fine tuning, playback region, loop, gain and
   the sample it plays; `SampleMap` is an ordered list of zones; a prepared sample is PCM
   with its frame count, channel count, source rate and content digest, held once per plan
   as the tunings are and referenced by a slot. The IR is built by whoever lowers and carries
   the bytes; the persisted asset behind them is Phase 10A's. Selected.
2. **A single-sample node with inline audio and no zone.** Rejected by the master plan's own
   words: it is the "special incompatible model" a later multisampler would replace.
3. **Decide the persisted `SampleAsset` now.** Rejected: nothing in Phase 6 persists a V2
   sample, `PROCESS.md` says not to commit a persisted form before its consumer, and the one
   constraint the prepared boundary places on it — a content digest and a preparation
   profile as the prepared key — is stated below without the rest.

**Q2 — how a sampler starts and ends on a note.**

1. **A declared trigger destination, written by the note's expansion.** `SOUND-INV-021`'s
   expansion already writes every declared pitch and velocity destination in the played
   scope; a kind may also declare a **trigger** destination, and the expansion writes the on
   edge to it at the note-on and the off edge at the release, each at its render position.
   The sampler is not playable — it has no `note_control` — so one scope still holds one
   address and `AmbiguousNoteScope` is untouched. Selected: it is the existing binding with
   one more destination class, it gives the sampler both edges so V1's release fade is
   reproducible, and a note sent to a sampler is refused as a note to any non-playable node is
   today: no note slot exists for it.
2. **Several playable nodes per scope, the gate landing on all.** Rejected: it reopens
   `SOUND-INV-016`'s "a note plays one node" and the refusal that keeps two instruments'
   nodes apart, for a case option 1 covers.
3. **An onset only, no off edge.** Rejected: V1's `Sustain` mode fades the sample on release
   inside the player, ahead of the envelope; without the off edge that behaviour is not
   representable and every sustained sampler note would be marked unrepresented.

**Q3 — V1's playback law, and how much of it Phase 6 builds.**

1. **V1's law where a test can hold it, the rest refused by name.** Rate is the resolved
   frequency over the root frequency times the fine-tune factor, the root resolved through
   the scope's own prepared tuning; interpolation is V1's two-tap linear with V1's loop wrap
   and crop clamp; a stereo sample is downmixed as V1 downmixes; `Sustain`, `OneShot` and
   `Loop` forward, with V1's 512-frame release fade in `Sustain`; a start offset into the
   region. `Reverse` and `PingPong` are refused by name until a consumer reaches them, and
   the lowering of a saved sampler module waits for the first sampler corpus case. Selected.
2. **V1 complete**, reverse and ping-pong included. Rejected for now: no fixture reaches
   them, and each is a playback-direction state machine the purity scan must then cover.
3. **A cleaner law** — no hard-coded fade, stereo through, a higher-order interpolator.
   Rejected: it is a delivered-behaviour change to every saved sampler patch, decided before
   a single one can be measured.

## Decision

1. **A `SampleZone` is the unit of mapping and a `SampleMap` is an ordered list of zones.**
   A zone carries: a **key range** (inclusive keyboard positions), a **velocity range**
   (inclusive normalized magnitudes), a **root key** (a keyboard position), a **fine tuning**
   in cents, a **playback region** (start frame, exclusive end frame, within the sample), an
   optional **loop** (start, exclusive end, within the region), a **gain** (a linear level),
   and the **prepared sample** it plays. Every field is a validated newtype; a region or
   loop outside the sample, an empty range, a loop outside its region, or a non-finite value
   is refused at IR construction, not clamped.
2. **Phase 6 builds the one-zone subset and refuses the rest by name.** A map holding more
   than one zone is refused at admission as `MapBeyondOneZone`; the types admit `N` so that
   a later multi-zone slice extends the selection and changes no type. A note whose key or
   velocity falls outside the one zone's ranges plays nothing and is counted, not refused: a
   range is the zone's, and silence is what an unmatched key is in every sampler.
3. **A prepared sample is immutable PCM with its shape and digest**: interleaved `f32`
   frames, a channel count of one or two, a frame count, the source sample rate, and a
   content digest computed at preparation. It is prepared off the audio thread, held **once
   per plan** in a plan-side table beside the prepared tunings, referenced by a
   `SampleSlot` that resolves to one array index on the audio thread, deduplicated by
   comparing the frames rather than the digests, and charged once to
   `prepared_immutable_bytes` plus one slot per reference. Two zones naming one sample share
   one table entry. The IR carries the prepared sample by reference (`Arc`), so `N` voice
   instances share it as they share every prepared record (`SOUND-INV-025`).
4. **The persisted asset is not decided here, and the prepared key binds it.** Phase 10A's
   `SampleAsset` must yield, for a given preparation profile, the prepared sample of clause
   3 keyed by source digest plus profile — "disposable cache, not canonical project audio",
   as the master plan says. Nothing in Phase 6 persists a V2 sample.
5. **The sampler is a voice-scope source kind that starts on a declared trigger.** It
   declares a **trigger** destination (`NoteMagnitude::Trigger`), a **pitch** destination and
   a **velocity** destination, all sample-positioned; it declares no `note_control` and is
   therefore never a note's address. `SOUND-INV-021`'s expansion writes the trigger's on
   edge with the note-on and its off edge with the release, at their render positions. A
   note naming the sampler itself is refused, as a note naming any non-playable node is.
6. **The rate law is V1's, through the tuning.** Playback rate is the frequency the scope's
   prepared tuning resolves for the note's key, over the frequency the same tuning resolves
   for the zone's root key, times `2^(fine_cents / 1200)`; the root is resolved at
   preparation and held with the zone. Under twelve-tone equal temperament this is V1's
   ratio; under any other tuning it is what V1 could not do.
7. **Playback is V1's**: two-tap linear interpolation with the second tap wrapped into the
   loop start at the loop boundary and clamped at the region end; a stereo sample summed to
   mono as `(left + right) × 0.5`; the zone's gain applied per frame; a **start offset** as
   a normalized position into the region, in force at the on edge. Three **play modes**:
   `OneShot` plays the region out and ignores the off edge; `Sustain` plays once and, on the
   off edge, fades linearly over 512 frames; `Loop` repeats the loop until the off edge and
   then fades as `Sustain` does. `Reverse` and `PingPong` are refused by name at admission.
8. **Velocity is a third destination under ADR-0059's rule**: the sampler's own
   `velocity_sensitivity` scales its output by V1's `(1 − s) + s × v`, computed in its kernel
   from the velocity it holds and the sensitivity it reads per frame.
9. **No per-note allocation.** A voice instance holds one player state — position, direction
   of the fade, remaining fade frames, held velocity and held rate — sized at preparation;
   the on edge resets it. The purity scan covers the kernel as it covers every other.
10. **Lowering a saved sampler module waits for the first sampler corpus case**, which Phase
    0B's bundle fixtures own. Until then `ModuleType::Sampler` is refused by the lowerer as
    today, and this record fixes what that lowering will map onto: V1's `SampleMeta` root,
    crop and loop become the zone's root key, region and loop; a missing root is C4 as V1
    defaults it; the key and velocity ranges are full; the gain is unity; V1's `level` is
    the kind's amplitude; `velocity_sensitivity`, `fine_tune`, `start_offset` and `play_mode`
    map to their namesakes; `direction` other than forward is refused by name.

## Falsifier and stopping rule

Violated if a sampler node allocates on a note, if a prepared sample is held more than once
per plan or charged other than once, if a kernel converts a key to a frequency, if a
sampler becomes a note's address, if the rate under twelve-tone equal temperament differs
from V1's ratio for any key, if the interpolation, downmix or fade differs from V1's for a
fixture that can measure it, or if a map with two zones is admitted. A different refusal
name, or a later slice's multi-zone selection, is not a defect.

## Consequences and risks

- **Accepted cost.** A third destination class in `SOUND-INV-021`'s expansion, one more
  plan-side prepared table with its own slot type, and a kernel with a small state machine
  (playing, fading, finished) whose every branch the purity scan enumerates.
- **Risk: the root frequency under the lowered tuning.** V1 computes the root with its own
  twelve-tone formula; V2 reads it from the prepared table the lowerer builds. If the two
  formulas differ by a rounding, every lowered sampler note is off by that rounding. Control:
  `P06-S005` asserts the prepared twelve-tone table's values against V1's
  `MidiNote::to_frequency` for every key, bit for bit, and records any difference as a
  measured figure rather than a tolerance; the first sampler corpus case measures the rest.
- **Risk: velocity thrice.** A lowered V1 sampler patch applies velocity in the sampler, the
  envelope and the output scaler, as V1 does. That is V1's behaviour and is reproduced, not
  corrected; a user who finds it excessive turns a sensitivity down in the project.
- **Revisit condition.** The first multi-zone consumer, which extends clause 2's selection;
  Phase 10A's `SampleAsset`, which must meet clause 4.

## Specification update

Acceptance adds the zone and map model, the prepared sample and its charge, and the trigger
destination to the Sound Core render contract as `SOUND-INV-026`, written by `P06-S005`;
`SOUND-INV-021`'s "exactly two magnitudes" becomes "two magnitudes and, where a kind declares
one, a trigger", and its expansion clause gains the off edge. `LOWER` gains nothing until the
first sampler corpus case.

## Review

Design consultation: put to the user on 2026-09-06 with the three questions above; to be
recorded.

Independent semantic reviewer: to be recorded at acceptance.

Stopping rule: a per-note allocation, a second address in a scope, a key converted in a
kernel, a prepared sample held twice, or a rate that is not V1's ratio under twelve-tone
equal temperament blocks acceptance. A refusal name or a later selection order does not.
