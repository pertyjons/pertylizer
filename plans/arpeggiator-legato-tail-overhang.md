> **Status: RESOLVED — but this plan MISDIAGNOSED the root cause (2026-07-03,
> branch `fix/arp-legato-tail-overhang`).** The audible "spurious +12 octave tone
> after the last note" was NOT an arp scheduling over-hang. It was a **voice-
> allocator** bug: on a MONO instrument the sequencer's cross-pitch legato coalesce
> emits one legato `NoteOn` per step with no matching `NoteOff`, and
> `VoiceAllocator::note_on_expr` pushed *every* pitch onto `held_notes`. A `[0,12]`
> figure accumulated both root and +12; the figure's single final `NoteOff(root)`
> removed only the root, so mono note-off fell back to the still-"held" +12 and
> **re-gated it into a note that rings forever**. Confirmed by a minimal allocator
> repro test (stuck `Active` voice on the octave) and fixed in
> `crates/synth_engine/src/voice_allocator.rs` (a legato `NoteOn` drops the
> departed active-voice pitch from `held_notes` instead of stacking it). Verified
> live in-app: the leftover is gone.
>
> The empirical §3 dump (`NoteOff{tick:240}` vs note end 210) was real but a *red
> herring* — offline render tears voices down at the window edge, so the stuck
> voice never sounded there, which is why the octave read as "live-only." The §4
> **clamp was REVERTED**: it addressed a different, subtle off-grid tail and, worse,
> shortened the last arp step — complexity with no observed benefit once the real
> bug was found. **KEPT:** the §2.1 freeze walk-boundary fix (`note_reach` reaches
> the note's end) as an independent, clearly-correct baking fix, plus its test.
> §2.2 (freeze BPM source) remains untouched and moot.

# Arpeggiator legato tail over-hang: an "off tone after the last note" in live playback

## Summary

An instrument carrying an **Arpeggiator NoteProcessor** (mode `Custom`, offsets
`[0, 12]`, rate `MilliHz 50000`, `octaves 1`, `legato true`, `gate 1.0`) sounds a
spurious high tone (the `+12` octave step) **after** a pattern's last note in
**live playback** — both `seq_play` (arrangement) and pattern-preview — while the
**same project rendered offline via `render_to_wav`** is clean, and **baking the
arp to explicit notes** (removing the processor) is also clean.

The off tone is a real, code-level over-hang: the arp's **last legato step is
scheduled to ring past the source note's end**, and live playback keeps the
transport running so it sounds, whereas the offline render truncates it at the
window boundary. This was found while exporting SID tunes to `.pertyproj`
(Rob Hubbard octave-arp stabs — Monty on the Run V1 Stab #5 / V3 Stab #6); the
`SID_NO_ARP_PROCESSOR` baked export A/B confirmed the processor is the culprit.

## 0. Correction to the intuitive hypothesis

Offline render does **not** freeze/bake processors. `render_to_wav_impl`
(`crates/pertylizer/src/mcp_bridge.rs:10251`) → `render_analysis_window`
(`:10351`, clones the song only for instrument isolation at `:10368`, never calls
freeze) → `render_arrangement_to_buffer_with_song`
(`crates/pertylizer/src/audio/arrangement_render.rs:179`) →
`OfflineEngineSession::render_range` (`:383`) builds a real `SynthEngine` and
drives the **same** `SequencerEngine`, so offline runs the **same** dynamic
`Arpeggiator::process_at_tick` as live. `freeze_note_processors` is a separate,
explicit, destructive tool (`mcp_bridge.rs:2128`, GUI button
`gui/sequencer/note_fx.rs:256`) — not part of any render. So offline ≠ live is
**not** "frozen vs dynamic."

## 1. Root cause (with anchors)

### 1a. The arp schedules a last step that outlives the source note
`crates/synth_sequencer/src/note_processor.rs`:
- Config: `:508-565` (`legato`, `gate`, `rate`, `custom`).
- Custom branch → `emit_custom`: `:742-780`, `:879-908`.
- `step_onset` (MilliHz → per-step duration): `:675-726`. With `gate = 1.0`,
  `duration = round(step_len * gate).clamp(1, gap) ≈ step_len` (`:703`).
- `step_note` stamps **every** step `legato = true` with a full step-length
  duration: `:928-939`.
- `chord_source` gates emission on `note.is_playing_at` (end-exclusive,
  `latch = false`): `:591-614`, and `note.is_playing_at`
  (`crates/synth_sequencer/src/note.rs:372-380`).

Worked trace (BPM 125 → `step_len = 16000·125/50000 = 40` ticks; offsets `[0,12]`):
a held source note ending at a tick that is **not** a multiple of `step_len`
(e.g. `end = 210`) has its last in-bounds step onset at tick `200` with offset
`12` (`root+12`, the octave), `legato = true`, `end_tick = 200 + 40 = 240` —
the octave voice is scheduled to ring **30 ticks past the note's end**.

### 1b. The coalesce emits a legato NoteOn with no paired NoteOff
`crates/synth_engine/src/sequencer_engine.rs`, `collect_events_at_tick`
cross-pitch legato coalesce `:805-874`: for a `legato` successor it **extends the
active note's `end_tick` and emits `NoteOn{legato:true}` with NO paired
`NoteOff`** (`:840-856`). The figure's only `NoteOff` comes from `check_note_offs`
when the final extended `end_tick` is reached (`:909-930`), fired after collect
each tick (`:566-569`). So the last step's `root+12` voice is released only at
`end_tick = 240`, after the source note ended at `210`. Audio-thread consumption:
`crates/synth_engine/src/synth_engine.rs:3466-3557` (`note_trigger`,
`route_sequencer_events`).

### 1c. Why offline hides it, live exposes it
`render_range` renders a bounded `[start, end)` window and simply **stops writing
samples** at the boundary — no loop, no trailing release
(`crates/pertylizer/src/audio/arrangement_render.rs:571-588`, explicit
"No trailing Stop" note `:599-602`). The over-hang past the window is truncated
out of the WAV. Live keeps the transport running:
- `seq_play` advances past the figure; auto-stop only force-releases at song end
  (`sequencer_engine.rs:582-599`), so the tail sounds first.
- **pattern-preview** loops via `current_tick % length` (`:679-716`) and never
  takes a `release_all_notes_into` branch, so the hanging voice is fully audible
  and can bleed into the next loop.

## 2. Why baking / freeze reads clean — two code-grounded suspects (need runtime confirm)

`freeze_processors` bakes by walking ticks and re-inserting the identical
`ExpandedNote`s (`note_processor.rs:1647-1706`), so static reading says frozen
playback *should* reproduce the tail — yet baked A/B was clean. Two concrete
divergences, both worth a runtime check (and both real bugs regardless):
1. **Freeze walk boundary ignores note duration:** `note_reach` uses
   `n.start.0 + 1` (`:1662-1671`), so `walk_end = length.max(note_reach)`
   (`:1683-1687`) can under-cover a held note's late arp steps.
2. **Freeze uses a different BPM than live:** freeze resolves MilliHz with
   `tempo_at(Tick(0))` (`note_fx.rs:253`, `egui_backend.rs:4669`), live uses
   `cached_tempo = tempo_at(current_tick)` (`sequencer_engine.rs:627,696,781`).
   Under a tempo map these give different step grids → different last-step
   alignment (possibly no over-hang after baking). Align these regardless.

## 3. Confirmation step (before locking the fix)

Add a `SequencerEngine`-level unit test that dumps the `Vec<SequencerEvent>`
emitted across the failing note boundary (no audio needed) and asserts there is a
`NoteOn(root+12, legato)` whose `NoteOff` lands **past** the source note's end.
This pins whether the fix belongs in the arp's last-step duration (§1a) or in the
coalesce's handling of a legato voice whose successor never arrives (§1b).

## 4. Fix direction

Primary (most local): **clamp the last arp step's `end_tick` to the source note's
end** for a `legato` step whose successor will never arrive — in `step_note` /
`step_onset` (`note_processor.rs:675-726, 928-939`), or by having the coalesce
(`sequencer_engine.rs:840-856`) bound the extended `end_tick` to the source note
end when no further legato successor follows. Either way the `root+12` tail is
released at the note end, not `step_len` later — matching the truncated offline
render and the baked notes. Then fix the freeze BPM source (§2.2) as a separate
correctness item so baked and live grids match under a tempo map.

## 5. Test homes

- Generative semantics (Custom + legato + gate=1.0 + non-`step_len`-aligned end):
  `crates/synth_sequencer/src/note_processor.rs` `#[cfg(test)]`, next to the
  existing boundary test `arp_latch_holds_last_chord_through_gap` (`:2267-2308`).
- Engine event assertion (the §3 dump): in-crate `sequencer_engine.rs` tests.
- Live/offline boundary audio: `crates/pertylizer/tests/preview_integration.rs`
  (already asserts exact `note_off_frame` boundaries `:190-232`) and
  `crates/pertylizer/tests/arrangement_render_integration.rs`.
- Freeze plumbing: `crates/pertylizer/tests/mcp_note_processors.rs` (`:123`).
