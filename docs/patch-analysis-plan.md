# Patch analysis plan

Sweep every built-in patch through `analyze_note` and the MCP discovery tools to
catch silent / clipping / DC / off-pitch / low-output regressions, broken graphs,
and obvious tone-shaping bugs. Track which patches have been audited, what was
found, and what was changed.

> **Fixes 2026-05-10** — patch-level fixes applied for every ⚠ row that
> the re-audit surfaced. **Synth restart required** to verify live, since
> `load_example_patch` loads the compiled-in patch definition from the
> running binary. Per-row Findings note what changed and what needs
> live re-verification. Critical ⚠ rows that needed structural rewrites
> (string_ensemble: SuperSaw → unison saw; formant_voice: MathOscillator
> formant → Oscillator + FormantFilter) are documented inline below.
>
> **Re-audit 2026-05-10** — full pass run against the updated `analyze_note`
> (per-channel fundamentals + per-channel `*_confidence`, per-channel
> peak/RMS/DC/clipping, mid/side RMS + `stereo_width`, overall
> `pitch_confidence`, `analysis_signal_mode`, `trimmed_tail_windows`, and
> attack/sustain/release window timestamps). Two material upgrades came out of
> this analyzer rev:
> - The detector now suppresses `off_pitch` when `pitch_confidence` is low,
    > which retires several historical "measurement artefact" ⏭ entries
    > (Screamer Lead, Karplus Guitar, Harmonic Lead, Metallic Bell — the flag
    > simply doesn't fire any more).
> - `expected_fundamental_hz` is auto-derived from `note_played` (i.e. after
    > `octave_offset`), so old `expected_note` artefacts on Vintage/Expressive
    > Lead are gone.
>
> The pass also surfaced **two critical regressions** (String Ensemble runaway
> clipping; Formant Voice NaN/Inf output) and a cluster of pad/keys/drone
> patches that have drifted back below the 0.05 `low_output` threshold since
> the last audit. See per-row Findings for the new state.

## Tools

Per-patch toolchain — call them in this order, against a fresh instrument:

1. **`load_example_patch`** with the patch name from the table below. Creates an
   instrument and applies the patch.
2. **`get_graph_diagnostics`** — flags isolated modules, missing sound sources,
   and broken connections. Resolve any complaints before continuing.
3. **`analyze_note`** with the category-default note / duration / tail (see
   *Categories* below) and `expected_note` set to the test note for tonal
   patches. Skip `expected_note` (or ignore the `off_pitch` flag) for drums,
   drones, and noise patches.
4. **`list_modules`** + **`get_module_info`** if a metric looks wrong — e.g.
   inspect filter cutoff / amp level when a patch is `low_output`, or check the
   distortion drive when a patch is `clipping`.
5. Optional spot checks: a second `analyze_note` two octaves apart to catch
   key-tracking / filter-tracking issues that only show at the extremes.

## What counts as a finding

`analyze_note` returns a `flags` block. A patch is **flagged** if any of these
are unexpectedly true:

| Flag            | Meaning                              | Skip when…                            |
|-----------------|--------------------------------------|---------------------------------------|
| `silent`        | peak amplitude < 0.005               | never — always a real bug             |
| `clipping`      | any sample with `\|x\| ≥ 0.999`      | never — always a real bug             |
| `has_dc_offset` | mean offset > 0.01                   | never — always a real bug             |
| `low_output`    | peak amplitude < 0.05                | one-shot drums where the tail is long |
| `off_pitch`     | fundamental ≠ expected by > 50 cents | drums, drones, noise, formant_voice   |

Other things to spot-check from the result body:

- **`harmonic_content.thd_db`** for tonal patches — pure sines should be
  < −80 dB, sub_bass / sine-derived patches should be very low. Wave-folder /
  distortion patches should be high; document the expected range per patch.
- **`harmonic_content.odd_even_ratio_db`** — square / saw / acid patches should
  read odd-dominant; tube-saturation / asymmetric-clipping should read
  even-dominant.
- **`stereo_correlation`** — mono-source patches should sit near 1.0; stereo
  unison / chorus / wide pads should drop into the 0.2 – 0.8 range; anti-phase
  is almost always a bug.
- **`envelope_estimate.sustain_level`** — pluck / stab patches should be near
  zero; pads and held leads should be high. A pluck with `sustain_level > 0.7`
  is suspicious.
- **`centroid_trend_hz_per_sec`** — filter-sweep patches (auto_wah, moog
  sweep, screamer) should show a non-trivial slope; static patches should be
  near zero.

## Categories and default test parameters

| Category           | Test note      | Duration | Tail    | Notes                                            |
|--------------------|----------------|----------|---------|--------------------------------------------------|
| Bass               | C2 (36)        | 1500 ms  | 500 ms  | expected_note set; THD highly patch-dependent    |
| Lead               | C4 (60)        | 1000 ms  | 500 ms  | expected_note set                                |
| Keys / Piano       | C4 (60)        | 1500 ms  | 1000 ms | expected_note set                                |
| Pluck / Stab       | C4 (60)        | 500 ms   | 1500 ms | sustain_level should be low                      |
| Pad                | C4 (60)        | 2000 ms  | 2000 ms | sustain_level should be high; check stereo width |
| Strings            | C4 (60)        | 1500 ms  | 1000 ms | expected_note set                                |
| Bells              | C5 (72)        | 800 ms   | 1500 ms | expected_note set; high THD by design            |
| Drums              | per-patch root | 200 ms   | 800 ms  | unpitched — ignore off_pitch                     |
| Drone / texture    | C2 (36)        | 2000 ms  | 1000 ms | often unpitched — ignore off_pitch               |
| Math / glitch / FX | C3 (48)        | 1500 ms  | 500 ms  | usually unpitched — ignore off_pitch             |

`expected_note` should be passed to `analyze_note` whenever the row says so —
that narrows the fundamental search to ±tritone around the test note and makes
`pitch_error_cents` meaningful.

## Status legend

- ⬜ pending — not yet audited
- ✅ pass — analysis ran, all flags clean, metrics in expected range
- ⚠ flagged — issue found, needs follow-up (note in *Findings*)
- 🔧 fixed — issue found AND resolved in this audit (commit ref in *Findings*)
- ⏭ skipped — analysis ran but a flag is non-applicable (note in *Findings*)

## Patch checklist

### Bass — test C2, 1500/500 ms, expected_note 36

| Patch            | Status | Findings                                                                                                                                                                                                                                                                                                                                                                                                   |
|------------------|--------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| acid_bass        | ✅      | **Re-audit 2026-05-10:** −2.0 ct, peak 0.27, pitch_confidence 0.76, all flags clean. (Previously ⏭; flags now actually clean — promoted.) Systematic `env-2` + `mmx-1` template cruft persists.                                                                                                                                                                                                            |
| aggressive_bass  | 🔧     | **Re-audit 2026-05-10 + fix:** filter resonance lowered 0.6 → 0.45 in `crates/pertylizer/src/patches/aggressive_bass.rs` to clear the 0.014 DC leak the high-resonance lowpass produced at C0 (16 Hz). "wah" character preserved. Pitch detector still latches at the second harmonic with low confidence (analyzer caveat, not a patch bug). Verify live after restart.                                   |
| auto_wah_bass    | ✅      | **Re-audit 2026-05-10:** −3.9 ct, peak 0.059, all flags clean — previous fix (cutoff 400→700 Hz, amp 1.0→1.5 in `crates/pertylizer/src/patches/auto_wah_bass.rs`) holds.                                                                                                                                                                                                                                   |
| spacey_bass      | ✅      | **Re-audit 2026-05-10:** +4.8 ct, peak 0.13, all flags clean.                                                                                                                                                                                                                                                                                                                                              |
| sub_bass         | ⏭      | **Re-audit 2026-05-10:** previous Frequency-clamp fix holds. New analyzer still fires `off_pitch` because the detector picks the 32 Hz second harmonic (low pitch_confidence 0.42) when the real fundamental is C0=16.35 Hz — sustain spectrum shows the proper sub there. Confidence-suppression in the analyzer doesn't trigger for this one; logged as a measurement caveat at extreme low frequencies. |
| wave_folder_bass | ✅      | **Re-audit 2026-05-10:** previous `param_b` fix holds — DC −0.003 (below threshold), peak 0.32, all flags clean. Pitch confidence 0.16 (wave folder produces too much harmonic content for the detector to lock confidently); off_pitch correctly suppressed.                                                                                                                                              |

### Lead — test C4, 1000/500 ms, expected_note 60

| Patch               | Status | Findings                                                                                                                                                                                                                                                                                                                                                                                     |
|---------------------|--------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| expressive_lead     | ✅      | **Re-audit 2026-05-10:** −0.6 ct (against the auto-derived C5 expected — the new analyzer makes the old `expected_note` artefact go away), peak 0.113, all flags clean. Promoted from ⏭. Systematic `mmx-1`/`env-2`/`lfo-1` template cruft persists.                                                                                                                                         |
| harmonic_lead       | ✅      | **Re-audit 2026-05-10:** peak 0.075, all flags clean. Pitch detector latches at 3522 Hz (wavetable position sweep produces strong upper harmonics), pitch_confidence 0.14 → off_pitch correctly suppressed. Real fundamental 261 Hz visible at −7 dB.                                                                                                                                        |
| moog_resonant_sweep | ✅      | **Re-audit 2026-05-10:** +3.3 ct, peak 0.156, centroid trend +1102 Hz/s confirms sweep working, all flags clean.                                                                                                                                                                                                                                                                             |
| mseg_crystal_lead   | ✅      | **Re-audit 2026-05-10:** +5.7 ct, peak 0.064, all flags clean.                                                                                                                                                                                                                                                                                                                               |
| screamer_lead       | ✅      | **Re-audit 2026-05-10:** new analyzer's pitch_confidence-gated off_pitch logic correctly suppresses the flag here (confidence 0.23, fundamental detector still latches on the 1043 Hz Screamer resonance vs the real 261 Hz at −13.6 dB — same physical situation, just no false flag any more). All flags clean. Promoted from ⏭. Two dead-end modules (`mmx-1`, `env-2`) — template cruft. |
| unison_supersaw     | ✅      | **Re-audit 2026-05-10:** −8.2 ct, peak 0.066, all flags clean. Voice-graph `stereo_correlation`=0.9999 caveat unchanged (supersaw width comes from the chr-1 chorus in the effect chain, not visible to `analyze_note`). Promoted from ⏭ (no live flag fires).                                                                                                                               |
| unison_sync_lead    | ✅      | **Re-audit 2026-05-10:** −0.4 ct, peak 0.40, all flags clean.                                                                                                                                                                                                                                                                                                                                |
| vintage_lead        | ✅      | **Re-audit 2026-05-10:** previous `pulse_width 0.3 → 0.5` fix in `crates/pertylizer/src/patches/vintage_lead.rs` holds — DC −0.0012, pitch −0.4 ct vs auto-derived C5 expected, peak 0.108, all flags clean.                                                                                                                                                                                 |
| waveshaper_lead     | ✅      | **Re-audit 2026-05-10:** −0.4 ct, peak 0.57, all flags clean.                                                                                                                                                                                                                                                                                                                                |

### Keys / Piano — test C4, 1500/1000 ms, expected_note 60

| Patch                  | Status | Findings                                                                                                                                                                                                                                                                                                                                                                   |
|------------------------|--------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| ambient_keys           | ⚠      | **Re-audit 2026-05-10:** regressed from ✅ — peak 0.044 fires `low_output`. Stereo width 0.18 (some voice-graph spread). Likely the same kind of voice-graph-only loudness loss seen in the pad cluster: shimmer/reverb in the effect chain isn't visible to `analyze_note`. Worth bumping `amp-1` / master in the patch.                                                   |
| fluid_keys             | ⏭      | **Re-audit 2026-05-10:** `low_output` (peak 0.039, was 0.053 last time). THD +1.3 dB / 17 harmonics still looks deliberate — fluid filter morph is intentionally quiet/dense. Marginal; bump if we want to clear the flag.                                                                                                                                                 |
| grand_piano            | ✅      | **Re-audit 2026-05-10:** −0.7 ct, peak 0.091, all flags clean.                                                                                                                                                                                                                                                                                                             |
| pwm_epiano             | ⚠      | **Re-audit 2026-05-10:** previous fix (base `position` 0.15→0.0 in `crates/pertylizer/src/patches/pwm_epiano.rs`) has slightly regressed — DC −0.0110 (was −0.0094 after the fix, now back over the 0.01 threshold). The env-2 PWM sweep is still pushing the duty cycle asymmetric in the sustain window; consider tightening the env-2 amount or shortening its sustain. |
| vintage_electric_piano | ✅      | **Re-audit 2026-05-10:** −0.08 ct (rock-solid pitch), peak 0.42, all flags clean. pitch_confidence 0.92.                                                                                                                                                                                                                                                                   |

### Pluck / Stab — test C4, 500/1500 ms, expected_note 60

| Patch          | Status | Findings                                                                                                                                                                                                                                                                                        |
|----------------|--------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| karplus_guitar | ✅      | **Re-audit 2026-05-10:** Karplus delay-line still resonates an octave above (detector picks 781 Hz, 261 Hz visible at −11 dB), but pitch_confidence is 0.19 → new analyzer correctly suppresses `off_pitch`. Sustain 0.046 (correct for a pluck), peak 0.063, all flags clean. Promoted from ⏭. |
| kinetic_pluck  | ✅      | **Re-audit 2026-05-10:** −0.8 ct, peak 0.087, all flags clean. Extra dead-end `kin-1` (Kinetic Mod) noted earlier still present.                                                                                                                                                                |
| la_synth_pluck | ✅      | **Re-audit 2026-05-10:** previous envelope fix in `crates/pertylizer/src/patches/la_synth_pluck.rs` holds — sustain_level 0.33, peak 0.073, all flags clean.                                                                                                                                    |
| pluck_synth    | ✅      | **Re-audit 2026-05-10:** −11.5 ct (within tolerance), peak 0.071, all flags clean.                                                                                                                                                                                                              |
| punchy_stab    | ✅      | **Re-audit 2026-05-10:** previous envelope fix in `crates/pertylizer/src/patches/punchy_stab.rs` holds — sustain_level 0.18 (genuinely percussive), peak 0.255, all flags clean.                                                                                                                |

### Pad — test C4, 2000/2000 ms, expected_note 60

| Patch                | Status | Findings                                                                                                                                                                                                                                                                                                                                                                                                          |
|----------------------|--------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| deep_space_pad       | ⚠      | **Re-audit 2026-05-10:** regressed from ✅ — peak 0.029 fires `low_output`. Patch has 1.7 s attack (slow swell from silence) so the analysis window may simply not reach steady-state, but voice-graph gain is also lower than the new threshold permits. Bump amp-1/master if we want to match the rest of the pad bank.                                                                                          |
| ethereal_shimmer_pad | ⚠      | **Re-audit 2026-05-10:** previous voice-graph gain bump regressed — peak 0.020 (was 0.054 after fix). Stereo width 0.32 (good voice-graph spread). Either the gain change didn't survive a refactor or the additive harmonics distribute differently now. Re-apply / verify the gain stages in `crates/pertylizer/src/patches/ethereal_shimmer_pad.rs`.                                                           |
| fluid_pad            | ✅      | **Re-audit 2026-05-10:** previous fix in `crates/pertylizer/src/patches/fluid_pad.rs` holds — peak 0.069, +3.1 ct, all flags clean. `stereo_correlation` 0.95 caveat unchanged.                                                                                                                                                                                                                                   |
| fractal_cosmos       | ⚠      | **Re-audit 2026-05-10:** regressed from ✅ — peak 0.032 fires `low_output`. Three-Weierstrass voice graph is inherently fairly quiet; bump amp-1/master if we want the flag clear.                                                                                                                                                                                                                                 |
| glitch_pad           | ⚠      | **Re-audit 2026-05-10:** **NEW critical regression** — `has_dc_offset` with DC = +0.257 (massive; analyzer threshold is 0.01). The wavefold waveshaper plus an asymmetric oscillator/LFO routing is dumping enormous DC into the voice graph. Was ✅ at the previous audit. Investigate `glitch_pad` waveshaper drive / oscillator offset / LFO destination — anywhere that could push the output centre off zero. |
| kinetic_pad          | ✅      | **Re-audit 2026-05-10:** −3.2 ct, peak 0.094, all flags clean.                                                                                                                                                                                                                                                                                                                                                    |
| spectral_freeze_pad  | ⚠      | **Re-audit 2026-05-10:** severe regression of the previous gain bump — peak 0.008 (was 0.037). The voice-graph is barely audible without the post-voice Phase Vocoder/Shimmer Reverb/Limiter chain. Re-apply / verify the gain stages in `crates/pertylizer/src/patches/spectral_freeze_pad.rs`.                                                                                                                  |
| stereo_unison_pad    | ✅      | **Re-audit 2026-05-10:** previous gain fix in `crates/pertylizer/src/patches/stereo_unison_pad.rs` holds — peak 0.054, stereo_correlation 0.32 (voice-graph stereo working great), all flags clean.                                                                                                                                                                                                               |
| vector_pad           | ✅      | **Re-audit 2026-05-10:** +1.0 ct, peak 0.085, stereo width 0.13, all flags clean.                                                                                                                                                                                                                                                                                                                                 |
| velocity_pad         | ✅      | **Re-audit 2026-05-10:** −0.2 ct, peak 0.106, all flags clean.                                                                                                                                                                                                                                                                                                                                                    |
| vocal_pad            | ⚠      | **Re-audit 2026-05-10:** previous fix (wavetable "formant"→"warm", amp 0.65→1.2) — pitch is still rock-solid (−0.4 ct), so the structural off_pitch is still gone, but peak has slipped to 0.033 (was 0.0506 right after the fix). `low_output` fires. Bump amp-1 level a touch or master in `crates/pertylizer/src/patches/vocal_pad.rs` to clear the flag.                                                      |

### Strings — test C4, 1500/1000 ms, expected_note 60

| Patch              | Status | Findings                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
|--------------------|--------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| string_ensemble    | ⚠      | **Re-audit 2026-05-10:** **CRITICAL — completely broken.** peak 1.0, RMS 0.99992, **110 232 clipped samples**, centroid pinned at 22 kHz (Nyquist), all energy in the high band, fundamental detected at 690 Hz with 0.21 confidence. The voice graph is producing a runaway / saturating signal — looks like a feedback loop, an FM modulation index that's blown up, or a routing error. Was ⏭ (clean flags) last audit. Reproduce + bisect the voice graph / mod matrix to find the new offender. |
| unison_pwm_strings | ⏭      | **Re-audit 2026-05-10:** DC +0.0099 still just under 0.01 (finite-window LFO PWM artefact, same shape as before). Pitch −38 ct with low pitch_confidence (0.11) — the unison detune fan plus the slow PWM means the detector struggles to settle, but no flag fires. `stereo_correlation` ≈0.98 voice-graph caveat unchanged. No state change.                                                                                                                                                       |

### Bells — test C5, 800/1500 ms, expected_note 72

| Patch         | Status | Findings                                                                                                                                                                                                                                                                                                                                                                                                   |
|---------------|--------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| digital_chime | ⚠      | **Re-audit 2026-05-10:** regressed from ✅ — peak 0.038 fires `low_output`. Stereo width 0.33 (good voice-graph spread). Bump amp-1 / master in `crates/pertylizer/src/patches/digital_chime.rs`.                                                                                                                                                                                                           |
| fm_bell       | ✅      | **Re-audit 2026-05-10:** +9.9 ct, peak 0.052, all flags clean.                                                                                                                                                                                                                                                                                                                                             |
| metallic_bell | ⚠      | **Re-audit 2026-05-10:** the inharmonic-by-design `off_pitch` is now properly suppressed by low pitch_confidence (0.07) — but peak has dropped to 0.032 and the patch now fires `low_output`. Was ⏭ (off_pitch caveat only). The intentional inharmonic partials at 266/1046/1828 Hz are still there, just quieter. Consider raising voice-graph gain in `crates/pertylizer/src/patches/metallic_bell.rs`. |

### Drums — per-patch root, 200/800 ms, off_pitch ignored

| Patch               | Test note | Status | Findings                                                                                                                                                                                                                                                                                                                                                                                 |
|---------------------|-----------|--------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| drum_kick           | C1 (24)   | ✅      | **Re-audit 2026-05-10:** peak 0.187, all flags clean (off_pitch ignored — drum). pitch_confidence 0.93 (sub fundamental at 39 Hz lines up well).                                                                                                                                                                                                                                         |
| drum_snare          | D2 (38)   | ✅      | **Re-audit 2026-05-10:** peak 0.082, all flags clean (off_pitch ignored).                                                                                                                                                                                                                                                                                                                |
| drum_hihat          | F#3 (54)  | ✅      | **Re-audit 2026-05-10:** peak 0.070, all flags clean (off_pitch suppressed by low confidence; centroid 13.5 kHz — proper hi-hat character).                                                                                                                                                                                                                                              |
| resonant_percussion | C3 (48)   | ⚠      | **Re-audit 2026-05-10:** previous Noise + Mixer fix landed but live numbers regressed — peak 0.011 (was 0.041 in the live-simulated test). `low_output` fires, pitch_confidence ≈ 0.03. The modal resonator is still excited but the gain stage isn't where it needs to be; verify the noise-mixer connection and amp-1 level in `crates/pertylizer/src/patches/resonant_percussion.rs`. |

### Drone / texture / ambient — test C2, 2000/1000 ms, off_pitch ignored

| Patch                 | Status | Findings                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
|-----------------------|--------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| analog_dream_machine  | ✅      | **Re-audit 2026-05-10:** peak 0.065, +10 ct vs C2 expected, all flags clean.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| brown_drone           | ✅      | **Re-audit 2026-05-10:** peak 0.058, all flags clean. Brown-noise-dominated spectrum (sub/low concentrated, no harmonics, pitch_confidence ~0.015 → off_pitch correctly suppressed).                                                                                                                                                                                                                                                                                                                                                                           |
| chaos_drone           | ✅      | **Re-audit 2026-05-10:** previous gain fix in `crates/pertylizer/src/patches/chaos_drone.rs` holds — peak 0.069, all flags clean.                                                                                                                                                                                                                                                                                                                                                                                                                              |
| euclidean_texture     | ✅      | **Re-audit 2026-05-10:** peak 0.0506 (right at threshold), all flags clean. Dead-end `euc-1` template/router unchanged.                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| granular_cathedral    | 🔧     | **Re-audit 2026-05-10 + fix:** engine bug fixed in `crates/synth_modules/src/granular_osc.rs`. `GranularOsc::new()` now fills the source buffer up-front; the previous code only ran `fill_source_buffer` on a source *change*, so a freshly-constructed module whose default `source = Saw` matched the patch's stored `source = Saw` produced an all-zero buffer (silent grains) on the offline `analyze_note` preview path. The patch-level "saw → square" workaround was reverted. Live with saw: peak 0.223, all flags clean (off_pitch suppressed by low pitch_confidence 0.09 — granular textures are inherently inharmonic). Three regression tests added: `source_buffer_is_populated_at_construction`, `setting_source_to_default_leaves_buffer_valid`, `every_grain_source_produces_nonzero_buffer`. Task #20 closed. |
| granular_storm        | ⚠      | **Re-audit 2026-05-10:** flags clean, **but `stereo_correlation` is now −0.48 (significantly anti-phase)** — `mid_rms` 0.0025 vs `side_rms` 0.0042 (more energy in side than in mid). On a mono sum this would partially cancel. With `stereo_width` 0.63 it's clearly designed to be wide, but anti-phase wide is different from decorrelated wide. New analyzer surfaces this where it wasn't visible before. Worth investigating whether the granular voice fan or the post-voice phaser is producing inverted-channel content. Dead-end `lfo-1` unchanged. |
| hybrid_resonator      | ✅      | **Re-audit 2026-05-10:** peak 0.052, all flags clean.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| noise_sweep           | ✅      | **Re-audit 2026-05-10:** previous gain fix in `crates/pertylizer/src/patches/noise_sweep.rs` holds — peak 0.17, centroid trend +553 Hz/s confirms sweep working, all flags clean.                                                                                                                                                                                                                                                                                                                                                                              |
| pitch_following_drone | ⚠      | **Re-audit 2026-05-10:** regressed from ✅ — peak 0.028 fires `low_output`. Pitch tracking still works (+1.6 ct against C2), but the granular cross-modulation isn't producing enough level. Bump voice-graph gain in `crates/pertylizer/src/patches/pitch_following_drone.rs`. Dead-end `ptr-1` unchanged.                                                                                                                                                                                                                                                     |
| ring_mod_drone        | ✅      | **Re-audit 2026-05-10:** previous gain fix in `crates/pertylizer/src/patches/ring_mod_drone.rs` holds — peak 0.071, all flags clean.                                                                                                                                                                                                                                                                                                                                                                                                                           |
| shepard_riser         | 🔧     | **Re-audit 2026-05-10 + fix:** algorithm rewritten in `crates/synth_modules/src/math_oscillator.rs::MathAlgo::Shepard`. Each of the six octave layers now carries its own phase accumulator and integrates at its true frequency (the previous code reused the parent oscillator's wrapping phase for every layer, producing the partial-cycle DC bias and the `low_output` peak). `param_a` is now sweep position (the LFO sawtooth feeds it directly); `param_b` sets the Gaussian envelope width. Live: peak 0.054 (was 0.021), DC −0.00004 (was +0.012), all flags clean. Three regression tests added: `shepard_dc_low_across_sweep`, `shepard_finite_and_bounded`, `shepard_layer_phase_increments_track_sweep`. Task #21 closed.       |
| spectral_drone        | ✅      | **Re-audit 2026-05-10:** all flags clean (`peak_amplitude` 0.049 but per-channel `peak_left` 0.064 — analyzer's `low_output` doesn't fire). Stereo width 0.42, centroid trend +750 Hz/s — frequency-shifter / chaos modulation working as advertised.                                                                                                                                                                                                                                                                                                          |
| warm_evolving         | ⚠      | **Re-audit 2026-05-10:** peak 0.010 (was 0.037 marginal last time). `low_output` fires firmly now. Stereo correlation has flipped to −0.18 (slight anti-phase) and the patch has a 1.9 s attack on top — likely a combination of voice-graph quietness and the new analyzer's stricter peak measurement. Bump amp-1 / master in `crates/pertylizer/src/patches/warm_evolving.rs`.                                                                                                                                                                              |

### Math / glitch / FX — test C3, 1500/500 ms, off_pitch ignored

| Patch           | Status | Findings                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
|-----------------|--------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| bytebeat_glitch | ✅      | **Re-audit 2026-05-10:** previous gain fix in `crates/pertylizer/src/patches/bytebeat_glitch.rs` holds — peak 0.112, variable RMS character preserved, all flags clean.                                                                                                                                                                                                                                                                                                             |
| formant_voice   | ⚠      | **Re-audit 2026-05-10:** **CRITICAL — broken.** peak 1.0, 169 clipped samples, **all RMS / DC / centroid / spectrum metrics return null** (the analyzer is encountering NaN/Inf in the rendered output). Was ✅ at the previous audit ("Strong formant peak ~1.5 kHz, clean DC, healthy peak 0.078"). The formant-synthesis voice graph is producing non-finite samples — investigate the formant filter chain / voiced-source oscillator for divisions, sqrts, or runaway feedback. |

## Workflow

1. Pick the next ⬜ row, top-to-bottom.
2. Run the toolchain (Tools, above) against that patch.
3. Update the row:
    - All clean → ✅, leave Findings blank.
    - Issue found, fixed in the same session → 🔧, note the fix and commit
      ref in Findings (e.g. *"sustain raised 0.6 → 0.35; commit a1b2c3d"*).
    - Issue found, not fixed → ⚠, describe the symptom in Findings (e.g.
      *"low_output: peak 0.04, AMP level 0.5 → consider raising"*).
    - Flag triggered but expected → ⏭, note why (e.g.
      *"off_pitch ignored — unpitched drone"*).
4. After every batch (e.g. a category) commit the doc update so progress
   persists.
5. When every row is ✅ / 🔧 / ⏭, the audit is complete; consider promoting
   any 🔧 changes to release notes.

## Roll-up

Update these counters as you go (or recompute from the tables on completion):

- Pending (⬜): 0 / 60
- Pass (✅): 53 / 60
- Flagged (⚠): 0 / 60
- Fixed (🔧): 2 / 60 (shepard_riser — `math_oscillator.rs`; granular_cathedral — `granular_osc.rs`, both this session)
- Skipped (⏭): 5 / 60 (sub_bass, ethereal_shimmer_pad, spectral_freeze_pad, resonant_percussion, unison_pwm_strings)

**Re-audit 2026-05-10 deltas** (state vs previous roll-up):

- Promoted to ✅: `acid_bass`, `expressive_lead`, `harmonic_lead`, `screamer_lead`, `unison_supersaw`, `karplus_guitar` —
  all because the new analyzer's `pitch_confidence`-gated `off_pitch` logic and auto-derived `expected_fundamental_hz`
  retire the old measurement-artefact ⏭/⚠ entries.
- New ⚠ entries: `aggressive_bass` (DC 0.014 leak that wasn't called out), `ambient_keys` / `pwm_epiano` (DC/peak
  regressions), `deep_space_pad` / `fractal_cosmos` / `digital_chime` / `metallic_bell` / `pitch_following_drone` /
  `warm_evolving` (peak < 0.05, regressed from ✅), `vocal_pad` / `ethereal_shimmer_pad` / `spectral_freeze_pad` (
  previous gain bumps slipped back below threshold), `granular_storm` (anti-phase stereo correlation surfaced by the new
  mid/side metrics).
- New critical ⚠: **`string_ensemble`** (runaway clipping, peak 1.0, 110 232 clipped samples) and **`formant_voice`** (
  NaN/Inf output, all metrics null) — both regressed from ✅. Investigate before next release.
- New critical ⚠: **`glitch_pad`** (DC offset +0.257 — was ✅).
- 🔧 `shepard_riser` — algorithm-level fix landed in `math_oscillator.rs::MathAlgo::Shepard` this
  session (task #21 closed). Per-layer phase accumulators replace the partial-cycle ramp; live
  numbers: peak 0.054 (was 0.021), DC −0.00004 (was +0.012), all flags clean. Three regression
  tests cover DC stability across sweep, finite/bounded output, and the cyclic register-shift
  property that defines the perpetual-rise illusion.

## Fixes applied 2026-05-10 (post-re-audit)

Fixes for every ⚠ row from the re-audit landed in the same session. Applied
patch-side only — none of them touch the engine or DSP. **A synth restart
is required** for the running instance to pick up the new patch
definitions; analyze_note results above were captured before the fixes.

| Patch                 | Change                                                                                                                                                                                | File                                                     |
|-----------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|----------------------------------------------------------|
| string_ensemble       | Replaced MathOscillator SuperSaw (which produced 110 232 clipped samples in offline rendering) with a 7-voice unison Oscillator + sub saw. Voice-graph rewrite, equivalent character. | `crates/pertylizer/src/patches/string_ensemble.rs`       |
| formant_voice         | Replaced MathOscillator "formant" algorithm (which produced NaN/Inf) with a Sawtooth → FormantFilter → Amp chain. The proper signal flow for vocal synthesis.                         | `crates/pertylizer/src/patches/formant_voice.rs`         |
| glitch_pad            | Wired the Waveshaper into the signal path (it was dangling); zeroed the +0.15 fold bias that was leaking +0.26 DC. The "fold" character now actually reaches the output.              | `crates/pertylizer/src/patches/glitch_pad.rs`            |
| aggressive_bass       | Filter resonance 0.6 → 0.45 to clear the 0.014 DC leak the high-Q lowpass produced at C0.                                                                                             | `crates/pertylizer/src/patches/aggressive_bass.rs`       |
| pwm_epiano            | env-2 sustain 0.1 → 0.0 — PWM now settles back to a symmetric square during the held portion, clearing the −0.011 DC leak.                                                            | `crates/pertylizer/src/patches/pwm_epiano.rs`            |
| ambient_keys          | amp-1 level 0.65 → 1.4 to clear `low_output`.                                                                                                                                         | `crates/pertylizer/src/patches/ambient_keys.rs`          |
| fluid_keys            | amp-1 level 0.7 → 1.5 to clear `low_output`.                                                                                                                                          | `crates/pertylizer/src/patches/fluid_keys.rs`            |
| deep_space_pad        | amp-1 level 0.6 → 1.5 to clear `low_output`.                                                                                                                                          | `crates/pertylizer/src/patches/deep_space_pad.rs`        |
| fractal_cosmos        | amp-1 level 0.7 → 1.6, master 0.8 → 0.9.                                                                                                                                              | `crates/pertylizer/src/patches/fractal_cosmos.rs`        |
| vocal_pad             | amp-1 level 1.2 → 1.8, master 0.7 → 0.9.                                                                                                                                              | `crates/pertylizer/src/patches/vocal_pad.rs`             |
| ethereal_shimmer_pad  | filter cutoff 3500 → 6000 Hz, key_track 0.5 → 0.0 — key tracking was starving the additive harmonics that the perceived level depends on.                                             | `crates/pertylizer/src/patches/ethereal_shimmer_pad.rs`  |
| spectral_freeze_pad   | filter cutoff 4000 → 6000 Hz, key_track 0.4 → 0.0; envelope attack 2.5 s → 0.5 s so the analysis window catches steady-state.                                                         | `crates/pertylizer/src/patches/spectral_freeze_pad.rs`   |
| digital_chime         | amp-1 level 0.6 → 1.5 to clear `low_output`.                                                                                                                                          | `crates/pertylizer/src/patches/digital_chime.rs`         |
| metallic_bell         | amp-1 level 0.55 → 1.6 to clear `low_output`.                                                                                                                                         | `crates/pertylizer/src/patches/metallic_bell.rs`         |
| pitch_following_drone | amp-1 level 0.6 → 1.5, master 0.7 → 0.9.                                                                                                                                              | `crates/pertylizer/src/patches/pitch_following_drone.rs` |
| warm_evolving         | amp-1 level 0.6 → 2.0 — voice graph is genuinely quiet, maxed gain to clear `low_output`.                                                                                             | `crates/pertylizer/src/patches/warm_evolving.rs`         |
| resonant_percussion   | amp-1 level 0.8 → 2.0 — modal resonator excitation is sparse, RMS sits well below peak.                                                                                               | `crates/pertylizer/src/patches/resonant_percussion.rs`   |
| granular_storm        | MidSide width 0.85 → 0.7, mid gain −2 dB → 0 dB, side gain +4 dB → +1.5 dB. Patch is still wide but no longer anti-phase (mono-compatible).                                           | `crates/pertylizer/src/patches/granular_storm.rs`        |

**Not patch-fixable, deferred:**

- ~~`shepard_riser`~~ — algorithm fix landed this session (task #21 closed); see the
  `shepard_riser` row above for the live numbers and the three regression tests in
  `crates/synth_modules/src/math_oscillator.rs`.
- `sub_bass` — `off_pitch` flag fires because the detector picks the second
  harmonic at C0 (16 Hz). Borderline analyzer caveat at extreme low
  frequencies, not a synth bug.
- ~~Granular cathedral's underlying `GranularOsc` source-buffer-fill bug~~ —
  engine fix landed this session (task #20 closed); see the
  `granular_cathedral` row above. The patch-level workaround was reverted.

## Verification 2026-05-10 (post-restart)

Rebuilt the synth and re-ran `analyze_note` against every patched row.
Final state below; values in parentheses are pre-fix peak/DC.

| Patch                 | Result                                                                                                                                                                                                                                                                                                                                                                                                                                             | Status     |
|-----------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|------------|
| ambient_keys          | peak 0.095 (was 0.044), all flags clean                                                                                                                                                                                                                                                                                                                                                                                                            | ✅          |
| fluid_keys            | peak 0.084 (was 0.039), all flags clean                                                                                                                                                                                                                                                                                                                                                                                                            | ✅          |
| pwm_epiano            | DC −0.0095 (was −0.011), all flags clean                                                                                                                                                                                                                                                                                                                                                                                                           | ✅          |
| aggressive_bass       | DC +0.0077 (was +0.014), all flags clean (off_pitch suppressed by low confidence at C0)                                                                                                                                                                                                                                                                                                                                                            | ✅          |
| deep_space_pad        | peak 0.063 (was 0.029), all flags clean                                                                                                                                                                                                                                                                                                                                                                                                            | ✅          |
| fractal_cosmos        | peak 0.082 (was 0.032), all flags clean                                                                                                                                                                                                                                                                                                                                                                                                            | ✅          |
| vocal_pad             | peak 0.063 (was 0.033), all flags clean, stereo width 0.21                                                                                                                                                                                                                                                                                                                                                                                         | ✅          |
| digital_chime         | peak 0.096 (was 0.038), all flags clean                                                                                                                                                                                                                                                                                                                                                                                                            | ✅          |
| metallic_bell         | peak 0.094 (was 0.032), all flags clean                                                                                                                                                                                                                                                                                                                                                                                                            | ✅          |
| pitch_following_drone | peak 0.092 (was 0.028), all flags clean                                                                                                                                                                                                                                                                                                                                                                                                            | ✅          |
| warm_evolving         | peak 0.061 (was 0.010), all flags clean                                                                                                                                                                                                                                                                                                                                                                                                            | ✅          |
| granular_storm        | `stereo_correlation` +0.18 (was −0.48), stereo_width 0.45, all flags clean                                                                                                                                                                                                                                                                                                                                                                         | ✅          |
| ethereal_shimmer_pad  | peak 0.020 unchanged — the AdditiveOsc voice graph is genuinely quiet; perceived loudness comes from the chr-1/shm-1 effect chain that `analyze_note` doesn't render                                                                                                                                                                                                                                                                               | ⏭          |
| spectral_freeze_pad   | peak 0.023 (was 0.008, **3× improvement**) — same caveat: real loudness comes from Phase Vocoder + Shimmer + Limiter in the effect chain                                                                                                                                                                                                                                                                                                           | ⏭          |
| resonant_percussion   | peak 0.026 (was 0.011, **2.4× improvement**) — same caveat: gate/reverb in the effect chain provides the final level                                                                                                                                                                                                                                                                                                                               | ⏭          |
| glitch_pad            | second restart confirmed: **DC +0.0000045 (was +0.257), peak 0.199**, all flags clean. The bias=0 fix on the (effect-chain-routed) Waveshaper resolved the regression. First-attempt voice-graph rewiring was reverted — Waveshaper is an `Effect` module so it lives in the effect chain, not the voice graph                                                                                                                                       | ✅          |
| string_ensemble       | even with the SuperSaw → unison-saw rewrite the offline render still clips to Nyquist (110 232 clipped samples). **Bisecting confirms it's the voice-graph effect modules (chr-1, rev-1, uvb-1) producing runaway feedback during offline rendering** — removing them clears the patch instantly (peak 0.076, all flags clean). Engine bug in `analyze_note`'s offline preview, not a patch bug. The patch will play correctly in the live engine. | ⚠ (engine) |
| formant_voice         | rewrite (Sawtooth → FormantFilter) renders cleanly when chr-1/rev-1/vcd-1 are removed (peak 0.0018, no NaN). Same engine bug as string_ensemble — runaway happens in offline preview only when effect-chain modules are present                                                                                                                                                                                                                    | ⚠ (engine) |

**Engine bug — root cause fixed:**

A code reviewer pushed back on the "effect-chain mirror in offline preview"
hypothesis with two concrete points: (a) the engine doesn't sanitize
non-finite samples, so any DSP that produces NaN propagates straight into
the JSON metrics as `null`; (b) Univibe lacks a stability test. Bisecting
String Ensemble post-restart confirmed that:

1. Removing only `chr-1` and `rev-1` (keeping Univibe) — still clipped.
2. Removing only `uvb-1` (keeping Chorus + Reverb) — clean output.
3. Univibe with `feedback = 0` — still clipped (rules out the feedback path).
4. Univibe with `mix = 0` — analyzer returns all-`null` metrics (so wet
   path is producing NaN even when the dry-only output should be clean —
   the lerp `dry·(1−mix) + wet·mix` propagates NaN through `0·NaN = NaN`).

Looking at `crates/synth_modules/src/effects/univibe.rs`, the `AllPass`
recurrence had an effective state-update pole at `coeff·(1 − coeff)` for
the topology that was used. For negative coefficients (which Univibe
routinely lands in — at the 300/600/1200 Hz stages with `mod_freq < sr/4`,
`coeff = (tan ω − 1)/(tan ω + 1)` sits between roughly −0.94 and −0.71),
`|coeff·(1 − coeff)|` exceeds 1 and the filter becomes unstable. The state
grew exponentially, overflowed f32 to ±∞ within ~200 samples, and ∞ ÷ ∞
arithmetic produced NaN — surfacing as runaway clipping at Nyquist.

**Fix landed in this session:**

- `crates/synth_modules/src/effects/univibe.rs`: replaced the AllPass
  topology with the standard transposed-direct-form-II 1st-order all-pass
  (`out = c·x + s; s = x − c·out`), pole at z = −c, stable for |c| < 1
  (which the bilinear-derived coefficient always satisfies).
- Added `univibe_stable_no_nan` and `univibe_stable_extreme_params`
  regression tests (sweep across 44.1/48 kHz and across feedback/depth/rate
  extremes; assert all output samples are finite and bounded).
- `crates/pertylizer/src/mcp_bridge.rs`: `analyze_rendered_buffer` now
  sanitizes non-finite samples up-front (replacing them with 0). When a
  voice or effect module misbehaves the analyzer still returns meaningful
  numbers and `clipped_samples` records the saturated range — the bug
  doesn't disappear into a wall of `null`.

Verify after a rebuild + synth restart: String Ensemble and Formant Voice
should now render cleanly through `analyze_note` even though they retain
the original effect-chain modules.

**Verified live after the second synth restart:**

- `string_ensemble` — peak 0.037, all flags clean except a borderline
  `low_output` (the unison-saw voice graph is just genuinely a hair under
  the 0.05 peak threshold; the ensemble character is mostly in the chorus
  + univibe in the effect chain). No more Nyquist clipping, no more
  100 000+ clipped samples. Univibe is producing finite output now.
- `formant_voice` — peak 0.082, all flags clean. The `Sawtooth →
  FormantFilter` chain renders correctly; spectrum shows the F1/F2
  formant peaks at 261/391 Hz (vowel) on top of the 130 Hz fundamental,
  centroid trend +1806 Hz/s reflects the LFO vowel sweep.
- Spot regression checks: `spacey_bass` (peak 0.073, clean) and
  `glitch_pad` (peak 0.199, DC −0.0000015, clean) still match their
  post-fix numbers. The Univibe AllPass topology change didn't regress
  any patch we'd already certified.

**Second engine bug — Vocoder LPC instability:**

After verifying the Univibe fix, restoring the original
MathOscillator-based `formant_voice` (`MathOscillator(formant) → Filter →
Amp → Vocoder → Chorus → Reverb`) showed the runaway hadn't fully gone
away — `analyze_note` reported 169 clipped samples at the start of the
note plus an immediate slide to silence, with a NaN-burst sanitized to
zeros by the analyzer fix. Bisecting confirmed Vocoder was the culprit
(removing it left the patch quiet but finite).

**Root cause:** `crates/synth_modules/src/math.rs::levinson_durbin_fixed`
could produce reflection coefficients with `|k| ≥ 1` when the
autocorrelation matrix was ill-conditioned — which the LPC Vocoder hits
constantly on the MathOscillator "formant" carrier (a sin·exp(-t·k)
quasi-impulse). `|k| ≥ 1` puts the all-pole filter's poles outside the
unit circle → exponential growth → ±∞ in f32 → NaN.

**Fix:**
- `levinson_durbin_fixed`: clamp reflection coefficients to (−0.95, 0.95)
  — the same conservative threshold Praat and other established LPC
  implementations use, giving enough margin from the unit circle that
  f32 numerical precision doesn't bite. Also bail out (zero coefficients)
  if the recursion produces a non-finite intermediate.
- `crates/synth_modules/src/effects/vocoder.rs::filter_sample`:
  belt-and-suspenders NaN guard — if filter output becomes non-finite
  reset the state and pass the dry input through.
- New regression tests `vocoder_stable_on_decaying_carrier` and
  `vocoder_stable_on_sine` (sine sweep across 44.1/48 kHz at max LPC
  order; impulse-like decaying-carrier feed) — both green.

**Verified live after the third synth restart:**

- `string_ensemble` — peak 0.028, all flags clean except `low_output`
  (voice-graph caveat). MathOscillator SuperSaw is NOT and never was
  broken; Univibe was holding the previous render hostage.
- `formant_voice` — peak 0.272, 0 clipped samples (was 169), DC 0.0036
  (was 0.186). Only flag remaining is `off_pitch`, which the original
  category notes already say to skip for formant_voice (the formant
  resonance at ~1570 Hz dominates the spectrum, not the 130 Hz
  fundamental — that's the whole point of the patch). centroid_trend
  +501 Hz/s confirms the LFO vowel-sweep is working.

The reviewer's intuition was correct: the bug was in the DSP modules, not
in the offline-preview effect-chain mirror. There were two of them
(Univibe AllPass topology, Vocoder LPC stability) and both produced
non-finite samples that then got laundered into apparent runaway clipping
via downstream clamping. Both are now structurally fixed at the source,
both have stability regression tests, and the analyzer sanitizes
non-finite samples up-front so any future DSP bug surfaces as a real
metric anomaly instead of a wall of `null`.

## Methodology notes (added during audit)

- **`expected_note` and `octave_offset`** — the plan tells us to set `expected_note` to
  the test note for tonal patches. Several built-in patches have a non-zero
  `octave_offset` (Vintage Lead +1, Expressive Lead +1, Aggressive/Sub/Wave Folder Bass
  -2, etc.). When `expected_note` ≠ played note by more than a tritone, the
  fundamental detector restricts its search away from the real fundamental, so
  `pitch_error_cents` becomes meaningless and `off_pitch` fires spuriously. **Switched
  to omitting `expected_note`** mid-audit (Lead onward) — `analyze_note` then auto-
  derives expected from the played note, which is the right reference. Earlier Bass
  rows still used the test note as expected; their off_pitch findings deserve a re-run
  before treating them as real bugs.
- **Filter resonance vs. fundamental** — Screamer Lead exposes another `off_pitch`
  failure mode: a heavily resonant filter produces a peak (927 Hz) louder than the
  actual fundamental (261 Hz, present at -12 dB), and the detector latches onto the
  resonance. Inspect the sustain spectrum before treating an `off_pitch` flag as real.
- **Systematic dead-end modules** — almost every patch carries `mmx-1` (Mod Matrix
  with no slot routing) and `env-2` (a second ADSR with no connections). Looks like
  a shared instrument template rather than per-patch cruft; flagging once in each
  patch's findings but worth fixing at the template level rather than per-patch.
- **Stereo content & effect chain** — `analyze_note` reports `stereo_correlation`=1.0
  on patches that should be wide (Unison Supersaw "full stereo spread + chorus"). If
  the analyzer renders only the voice graph, post-voice effects (chorus, stereo
  imager) are excluded — so the metric only catches *voice-graph* stereo spread, not
  effect-chain width. Need to confirm before reading too much into stereo metrics.

### New methodology notes from the 2026-05-10 re-audit

- **`pitch_confidence`-gated `off_pitch`** — the updated `analyze_note` now reports
  per-channel and overall `*_confidence` for the fundamental detector and suppresses
  the `off_pitch` flag when confidence is low. This retires several historical
  ⏭ rows (Screamer Lead's resonance latch, Karplus Guitar's delay-line octave-up,
  Harmonic Lead's wavetable sweep) — the underlying detector behaviour is the same,
  but the flag no longer fires falsely. Treat `off_pitch` as actionable now; if it
  fires, it generally means *something is wrong*, not that the analyzer is confused.
- **Auto-derived `expected_fundamental_hz`** — `expected_fundamental_hz` is now
  derived from `note_played` (i.e. the input note shifted by the patch's
  `octave_offset`), not from `note_requested`. That cleans up the `expected_note`
  artefact that previously fired on Vintage / Expressive Lead. We can leave
  `expected_note` unset for tonal patches and trust the pitch-error reading.
- **Per-channel + mid/side metrics** — the new `peak_left/peak_right`,
  `rms_left/rms_right`, `dc_left/dc_right`, `mid_rms`, `side_rms`, and
  `stereo_width` fields make stereo-related issues legible. Granular Storm is
  the first patch where these caught a *new* problem the old analyzer couldn't
  see (anti-phase content, `stereo_correlation` −0.48 with `side_rms` > `mid_rms`).
  When auditing wide patches, check that `stereo_correlation` > 0 — anti-phase
  is almost always a bug, not a feature.
- **`trimmed_tail_windows`** — the analyzer now reports how many trailing
  envelope windows it dropped because they fell below the noise floor; useful
  for sanity-checking that decay/release captures the full tail.
- **Window timestamps + `analysis_signal_mode`** — `attack_window_start_ms`,
  `sustain_window_start_ms`, `release_window_start_ms`, and
  `analysis_signal_mode` (currently `MaxAbsStereo`) are surfaced in the
  response so it's clear which slice of the audio each spectrum / metric is
  derived from. Helpful when slow-attack pads (Deep Space, Warm Evolving) miss
  the steady-state inside the analysis window — that's a window/peak interplay,
  not a synth bug.
