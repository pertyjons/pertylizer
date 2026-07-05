# Pertylizer MCP feedback — running log

Running log of things missing / awkward in the `synth` (Pertylizer) MCP, collected while doing SID→patch timbre-matching in sid-analyzer. Per asked (2026-06-21) to keep this so the MCP can be improved.

> Source of truth: this file is a snapshot of Claude Code's session memory
> `~/.claude/projects/-home-per-github-sid-analyzer/memory/pertylizer-mcp-feedback.md`,
> which continues to accumulate new findings. Everything resolved through
> 2026-07-05 has been pruned — only OPEN items remain below.

## compare_spectra energy-masked distance over voiced target frames (option (b), open)

Wish option (a) — **report the voicing penalty as its own field** — SHIPPED (main
`30ed9caf`): `log_spectral_distance` is now the pure log-bin RMS and a new
`voicing_penalty_db` field carries the 60 dB mismatch penalty, so the primary
scalar no longer saturates on silent/unvoiced target windows. Combined score =
`log_spectral_distance + voicing_penalty_db`; steer ranking with `mel_l2_distance`
(never penalised) on silence-dominated material.

Still open — the deeper option **(b)**: an option to restrict the distance to
frames where the *target* has energy (voiced/energy-masked RMS). On sparse
staccato material (3 s V2-solo stab window, mostly silence) even the raw log-bin
RMS was ~identical across candidates (~50.3); the energy-masked measure scripted
externally with numpy ranked three candidates cleanly (73.3/80.1/87.3 dB) where
whole-window and 500 ms windows both failed. Needs frame-aware masking in
`compare()` (it currently aggregates one spectrum per source). Also worth a doc
line: short windows on staccato content are alignment-sensitive — envelope-align
first; for alternating sources (SID tri↔noise every 20 ms) compare a single
voiced frame or a carrier-only render.

## Ring-mod HF fidelity — oversampled ring/sync bus (open, 2026-07-02)

The native `sid` ring-mod's sideband POSITIONS are exact (330 Hz comb lands the
Nemesis 988±165 Hz sidebands to <1 Hz), but broadband `compare_spectra`
distance holds ~17 dB vs a reSID target: a ≈−4 dB deficit across the whole
3–9 kHz HF run. **Root cause pinned:** the ring edge is read at HOST rate (the
neighbour `msb` buffer is host-rate) = ~22.7 µs edge jitter even at 4×
oversample, smearing the HF sidebands ~3 dB; reSID does the XOR at 1 MHz. The
localized one-sided PolyBLEP fix (shipped, replaced the old linear-crossfade
`slewed_dac_sample` hack — worth keeping) halved centroid error but couldn't
close the broadband gap because the step *timing*, not its shape, is the
dominant error. **Real fix = an OVERSAMPLED ring/sync bus** (expose `msb` at
4×-rate or the sub-sample crossing) — a cross-module contract change. Per's
call: keep the BLEP and tackle the oversampled bus later, or revert. Filed as
ring's next step (see `plans/sid-oscillator-module.md` §11 ring subsection).

## Arpeggiator restarts phase per note-onset — breaks sub-cycle-length arps (2026-07-03)

Exporting Martin Galway (Ocean Loader) SID→pertyproj, tried to convert its fast chord
stabs from a per-frame bake to an `Arpeggiator` NoteProcessor (Custom offsets, MilliHz =
frame rate, legato, gate 1.0). Offline solo-render band-energy A/B: the processor
**systematically drops the trailing chord tones**. Ocean Loader stab offsets `[4,7,0]`
(root last), notes ~2 frames (40 ms) each; measured root (A#4) energy **36** with the
processor vs **931** baked (D5 1075/1921, F5 1772/5990) — the root is essentially gone.

**Root cause:** the SID arp has a **free-running phase** — the arp counter runs continuously
across successive stabs, so different stabs catch different phases and over a window all chord
tones sound. Pertylizer's `Arpeggiator` **restarts the offset cycle on every note onset**, so a
note shorter than one full cycle only ever plays the leading offsets and never reaches the
trailing ones. For a 3-step cycle on 2-frame stabs, the last offset (here the root) is never hit.

**Wish:** a **free-running / continuous-phase arp mode** on the `Arpeggiator` NoteProcessor —
the offset index derives from absolute transport position (or a per-instrument running counter)
rather than resetting at each note's start. That would let short-stab arps (the SID's normal
case) convert to a processor faithfully instead of forcing a per-frame bake. Anchors for a fix:
`emit_custom`/`step_onset` in `crates/synth_sequencer/src/note_processor.rs`.

**Workaround shipped in the exporter:** hybrid gate — only convert an arp plan to a processor
when every note spans ≥ `offsets.len()` frames (completes the cycle per onset); shorter stabs
keep the trace-faithful bake. So Galway bakes, Hubbard (longer stabs, offsets start at 0)
processes. Once free-running phase lands, drop the length gate for those plans.

## `render_to_wav` truncates trailing/hanging voices at the window edge (standing limitation)

Offline `render_range` renders a bounded window and truncates any voice still ringing past the
window edge, so `render_to_wav` + `analyze_*` + FFT cannot surface over-hang / tail artifacts
that live playback plays (this is how the legato-tail arp bug — since fixed — stayed invisible
to the offline diagnostic loop). **Wish:** either render a short release-tail past the window,
or a flag to capture/analyze the LIVE playback audio, so live-only artifacts are reproducible
via MCP.

## (add more below as found)
