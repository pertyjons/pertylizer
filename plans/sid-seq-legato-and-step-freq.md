# SID oscillator seq: per-step frequency (+ Note.legato doc fix)

Status: REVISED 2026-07-06 after review — the original part 1 ("legato
retunes restart the program") was a misdiagnosis; the reviewer was right
that the engine already has the semantics. Verified in-engine and fixed
export-side (sid-analyzer `f887883`). What remains for Pertylizer is one
small doc fix and the per-step frequency feature.

## 1. Note.legato doc/impl mismatch (doc fix only)

Verified engine chain (all present and correct):
`sequencer_engine.rs` boundary coalesce (a **legato successor** extends the
active voice cross-pitch, suppressing the NoteOff) → `legato: true` NoteOn →
`voice_allocator.rs::note_on_expr` (`trigger.legato` forces the no-retrigger
path regardless of Poly/Mono) → `allocate_mono` → `glide_to_note_expr` —
`graph.note_on` is never called, so the sid seq position survives the tie.
The engine's own test ties C4→E4 with the flag on **E4** (the successor).

But `synth_sequencer/src/note.rs:274` documents the field as "when set,
this note connects to its **successor** without re-gating" — i.e. flag on
the *predecessor*. sid-analyzer followed the doc, so every exported legato
tie re-gated the envelope and restarted the seq program — the artifact the
original plan blamed on the SID module. Export fixed (flag now on
continuations). **Ask: reword the `Note.legato` doc** to "when set, this
note is a legato continuation of its predecessor — it glides onto the
active voice without re-gating" so the next client doesn't repeat this.
(Alternatively flip the engine to predecessor-semantics, but the engine
behavior is shipped, internally consistent, and tested — the doc is the
odd one out.)

## 2. Per-step frequency for the waveform seq (the real feature)

Unchanged motivation: Hubbard drums jump the frequency register on noise
frames (V3 bass tick: body E-2 `$057B` ≈ 82 Hz, noise frame `$684C` ≈
1.6 kHz — 19×; V2 snares sweep `$3038 → $1078` through the run). The seq
clocks every step at the played pitch, so far-from-pitch noise steps render
dark/wrong: emitting the V3 program measured 9.9 → 12.6 dB (V3-solo,
time-resolved compare_spectra) and the export now suppresses programs whose
measured noise frequency is >4× from the note pitch
(`PROGRAM_NOISE_PITCH_MAX_RATIO`, sid-analyzer `export/synth.rs`).

Design, revised per review: keep it inside the SID module like
`SeqStep(i, mask)` — but **not** a `0 = track pitch` sentinel (`$0000` is a
valid freq register). Either:
- a per-step **enable bitmask** (`seq_freq_mask: u16`, bit i = "step i uses
  `seq_step_freq_i`"), or
- an out-of-range sentinel (step value `65536` = track pitch) since the
  params are f32 anyway.
The bitmask is the cleaner schema story (16 freq params + one mask, all
0-default = today's behavior).

## Exit gate

- Doc: `Note.legato` comment states continuation semantics.
- Per-step freq: the AWM V3 bass-tick program (P N P, N pinned `$684C`)
  beats today's suppressed-program baseline (9.9 dB, V3-solo 38–44 s,
  time-resolved compare_spectra); export drops the ratio gate.

## Post-review measurement caveat (sid-analyzer side, for the record)

With the export flag flipped, the AWM drums window measured neutral-to-
slightly-worse (full mix 9.18→9.30, V2-solo 29.0→31.8) — expected
confounds: the window's reference is noise-dominant while correct ties
sustain more tonal energy, and the tune's V1 lead is still missing from the
export (native-decode truncation, queued). Possible residual engine factor
worth checking while in here: the known **arp legato-tail overhang** (voice
energy past note end) now applies to every tied figure, not just arps.
Re-A/B after the V1 fix; the flip stands on engine-code grounds.
