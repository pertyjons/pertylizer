#!/usr/bin/env python3
"""EVD-0013 control C3: the oscillator difference floor, per octave band.

V1's `Waveform::Sine` and V2's `Sine` differ in **two** ways, and a bound that
covers only the first is not a bound:

- the waveform function — `fast_sin_turns`, a parabolic approximation with a
  correction term (`crates/synth_modules/src/math.rs:390`), against `f64::sin`
  (`crates/synth_engine_v2/src/node/kernels.rs:519`); and
- the phase accumulator — V1 advances an `f32` phase by an `f32` increment and
  wraps it through `Phase::new`'s `rem_euclid` (`crates/synth_core/src/types/
  normalized.rs:520`), while V2 accumulates and wraps in `f64`.

This script reproduces **both** accumulators and **both** waveform functions
outside either engine, runs each result through the filter the fixture applies,
and integrates the residual into the same octave bands `SpectrumDifference`
compares. What it prints is the per-band bound EVD-0013's E3 attributes to the
oscillator: a band difference larger than its bound is not the oscillator, and
needs its own cause.

It renders nothing and needs neither engine.

    python3 plans/v2/evidence/phase-02/evd_0013_oscillator_floor.py [frequency_hz]

The optional argument is E3a's sweep point; it defaults to the fixture's 440 Hz.
The bound is a function of frequency, so each sweep point needs its own run.

Both arms are gated by the **same** envelope, because the amplifier sits after
the filter and multiplying by a time-varying envelope moves residual energy
between bands — an ungated residual would not bound a gated render. What the
bound therefore does **not** cover is the *difference* between the two engines'
envelope shapes, which is E2's subject and a separate cause.
"""

import sys

import numpy as np

F32 = np.float32

# The fixture, from EVD-0013.
SAMPLE_RATE = 44100
FUNDAMENTAL_HZ = 440.0
CORNER_HZ = 1000.0
# `Resonance::BUTTERWORTH` is `FRAC_1_SQRT_2` rounded to f32, which is what the
# crate stores and what the coefficients are derived from.
QUALITY = float(F32(2.0**-0.5))
# The corpus render window for CORPUS-0001: 2 s plus a 1 s tail, and the note
# held for the whole visible range.
SECONDS = 3.0
NOTE_SECONDS = 2.0

# The fixture's envelope, from EVD-0013.
ATTACK_S = 0.010
DECAY_S = 0.100
SUSTAIN = 0.700
RELEASE_S = 0.200

# `octave_band_energies`, from crates/pertylizer/src/audio/analysis/mod.rs:737.
BAND_EDGES_HZ = [
    20.0, 40.0, 80.0, 160.0, 315.0, 630.0,
    1250.0, 2500.0, 5000.0, 10000.0, 20000.0,
]
BAND_FRAME = 4096
HANN_POWER_GAIN = 0.375


def fast_sin_turns(turns: np.float32) -> np.float32:
    """`crates/synth_modules/src/math.rs:390`, in f32 at every step."""
    phase = F32(turns - np.float32(np.floor(turns)))
    x = phase if phase < F32(0.5) else F32(phase - F32(1.0))
    y = F32(F32(16.0) * F32(x * F32(F32(0.5) - F32(abs(x)))))
    return F32(y * F32(F32(F32(0.225) * F32(abs(y))) + F32(0.775)))


def v1_oscillator(frames: int) -> np.ndarray:
    """V1's sine: f32 phase, f32 increment, `Phase::new`'s rem_euclid wrap.

    `Frequency::phase_increment` is `self.0 / sample_rate.0` in f32, and
    `Oscillator::generate_sample` reads the phase **before** advancing it, so
    frame 0 is phase 0.
    """
    increment = F32(F32(FUNDAMENTAL_HZ) / F32(SAMPLE_RATE))
    out = np.empty(frames, dtype=np.float32)
    phase = F32(0.0)
    for index in range(frames):
        out[index] = fast_sin_turns(phase)
        # Phase::new(value) is value.rem_euclid(1.0), in f32.
        phase = F32(np.float32(F32(phase + increment)) % F32(1.0))
    return out


def v2_oscillator(frames: int) -> np.ndarray:
    """V2's sine: f64 accumulator, f64 sin, result rounded to f32.

    `seconds_per_frame` is `1.0 / f64::from(rate.as_f32())`
    (`crates/synth_engine_v2/src/node.rs:255`), and the wrap subtracts the floor
    in both directions.
    """
    increment = float(F32(FUNDAMENTAL_HZ)) * (1.0 / float(F32(SAMPLE_RATE)))
    out = np.empty(frames, dtype=np.float32)
    phase = 0.0
    for index in range(frames):
        out[index] = F32(np.sin(2.0 * np.pi * phase))
        phase += increment
        if not 0.0 <= phase < 1.0:
            phase -= np.floor(phase)
    return out


def low_pass(signal: np.ndarray) -> np.ndarray:
    """The recurrence both engines run, with V1's f32 coefficients.

    Both oscillator signals go through the **same** filter, so what survives is
    the oscillator's difference propagated through the stage the render applies
    — not the filters' own difference, which EVD-0013 accounts for separately.
    """
    g = F32(np.tan(np.float32(np.pi * CORNER_HZ / SAMPLE_RATE)))
    k = F32(2.0 - 2.0 * F32((2.0 - 1.0 / QUALITY) / 2.0))
    a1 = F32(F32(1.0) / F32(F32(1.0) + F32(g * F32(g + k))))
    a2 = F32(g * a1)
    a3 = F32(g * a2)

    out = np.empty_like(signal)
    ic1 = F32(0.0)
    ic2 = F32(0.0)
    for index, sample in enumerate(signal):
        v3 = F32(sample - ic2)
        v1 = F32(F32(a1 * ic1) + F32(a2 * v3))
        v2 = F32(F32(ic2 + F32(a2 * ic1)) + F32(a3 * v3))
        ic1 = F32(F32(2.0) * v1 - ic1)
        ic2 = F32(F32(2.0) * v2 - ic2)
        out[index] = v2
    return out


def envelope(frames: int) -> np.ndarray:
    """V2's four-segment envelope: linear ramps of an exact frame count.

    The **same** envelope multiplies both arms. Gating matters here because the
    amplifier sits after the filter, so what E3 measures is a gated signal, and
    multiplying by a time-varying envelope redistributes residual energy between
    octave bands — an ungated residual is not a bound on a gated one. Holding one
    envelope across both arms is what keeps the residual the oscillator's: the
    two engines' envelope *shapes* differ, and that difference is E2's subject,
    not this control's.
    """
    attack = int(ATTACK_S * SAMPLE_RATE)
    decay = int(DECAY_S * SAMPLE_RATE)
    release = int(RELEASE_S * SAMPLE_RATE)
    gate_off = int(NOTE_SECONDS * SAMPLE_RATE)

    out = np.zeros(frames, dtype=np.float32)
    held = np.arange(min(gate_off, frames))
    out[held] = np.where(
        held < attack,
        held / attack,
        np.where(
            held < attack + decay,
            1.0 - (1.0 - SUSTAIN) * (held - attack) / decay,
            SUSTAIN,
        ),
    )
    if gate_off < frames:
        tail = np.arange(gate_off, min(gate_off + release, frames))
        level = out[gate_off - 1] if gate_off > 0 else 0.0
        out[tail] = level * (1.0 - (tail - gate_off) / release)
    return out


def band_rms(samples: np.ndarray) -> dict[float, float]:
    """`octave_band_energies` reproduced: Hann-windowed Welch, half overlap.

    Same frame, same overlap, same Parseval weighting and same Hann power gain
    as `crates/pertylizer/src/audio/analysis/mod.rs:784`, so a figure here is on
    the same scale as one in a `pertylizer compare` report.
    """
    if len(samples) < BAND_FRAME:
        return {}
    # Symmetric, matching `crates/synth_dsp/src/spectral.rs:38`, whose x is
    # i / (N - 1). NumPy's `hanning(N)` is that window; the periodic form
    # `hanning(N + 1)[:N]` is a different one.
    window = np.hanning(BAND_FRAME)
    bin_hz = SAMPLE_RATE / BAND_FRAME
    nyquist_bin = BAND_FRAME // 2
    sums = np.zeros(len(BAND_EDGES_HZ) - 1)
    frames = 0

    for start in range(0, len(samples) - BAND_FRAME + 1, BAND_FRAME // 2):
        spectrum = np.abs(np.fft.rfft(samples[start:start + BAND_FRAME] * window))
        for k in range(1, len(spectrum)):
            hz = k * bin_hz
            band = band_of(hz)
            if band is None:
                continue
            weight = 1.0 if k == nyquist_bin else 2.0
            sums[band] += weight * spectrum[k] ** 2
        frames += 1

    if frames == 0:
        return {}
    scale = 1.0 / (BAND_FRAME**2 * HANN_POWER_GAIN * frames)
    return {
        BAND_EDGES_HZ[band]: float(np.sqrt(sums[band] * scale))
        for band in range(len(sums))
        if sums[band] > 0.0
    }


def band_of(hz: float) -> int | None:
    if hz < BAND_EDGES_HZ[0] or hz >= BAND_EDGES_HZ[-1]:
        return None
    for band in range(len(BAND_EDGES_HZ) - 1):
        if BAND_EDGES_HZ[band] <= hz < BAND_EDGES_HZ[band + 1]:
            return band
    return None


def main() -> None:
    global FUNDAMENTAL_HZ
    if len(sys.argv) > 1:
        FUNDAMENTAL_HZ = float(sys.argv[1])
    frames = int(SECONDS * SAMPLE_RATE)
    v1 = v1_oscillator(frames)
    v2 = v2_oscillator(frames)

    # Sine -> Filter -> Amplifier(envelope), so the gate is applied after the
    # filter, and the same gate is applied to both arms.
    gate = envelope(frames)
    filtered_v1 = (low_pass(v1) * gate).astype(np.float32)
    filtered_v2 = (low_pass(v2) * gate).astype(np.float32)
    residual = (filtered_v1 - filtered_v2).astype(np.float32)

    print("# EVD-0013 control C3 — the oscillator difference floor, per band")
    print(f"sample_rate,{SAMPLE_RATE}")
    print(f"seconds,{SECONDS}")
    print(f"frequency_hz,{FUNDAMENTAL_HZ}")
    print(f"corner_hz,{CORNER_HZ}")
    print(f"max_absolute_residual_prefilter,{np.abs(v1 - v2).max():.6e}")
    print(f"max_absolute_residual_postfilter,{np.abs(residual).max():.6e}")

    reference = band_rms(filtered_v1)
    difference = band_rms(residual)

    print()
    print("low_hz,reference_dbfs,residual_dbfs,bound_db")
    for low_hz in sorted(reference):
        ref = reference[low_hz]
        res = difference.get(low_hz, 0.0)
        # The band delta this residual can produce, worst case: it either adds
        # to or subtracts from the reference in that band, so the larger of the
        # two directions is the bound.
        bound = max(
            abs(20 * np.log10((ref + res) / ref)),
            abs(20 * np.log10(max(ref - res, 1e-30) / ref)),
        )
        print(
            f"{low_hz:.0f},{20 * np.log10(max(ref, 1e-30)):+.2f},"
            f"{20 * np.log10(max(res, 1e-30)):+.2f},{bound:.4f}"
        )

    print()
    print("# `bound_db` is what E3a and E3b attribute to the oscillator in that")
    print("# band. A measured band difference larger than it is not the")
    print("# oscillator and needs its own cause. Both arms carry the same gate,")
    print("# so the envelopes' difference in shape is not in this bound.")


if __name__ == "__main__":
    main()
