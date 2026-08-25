#!/usr/bin/env python3
"""EVD-0015: quantum occupancy in real projects, over sampled admitted rates.

Run from the repository root:

    python3 -B plans/v2/evidence/phase-03/evd_0015_quantum_occupancy.py
"""
import glob
import json
import math
import os
from collections import Counter

PPQ = 960                                   # synth_sequencer: 960 PPQN
Q = 64                                      # ADR-0037's render quantum, in frames
CAP = 256                                   # max_events_per_quantum
# Six SAMPLED points in accepted_sample_rates' inclusive 8k-192k range, not the range
# itself. 8 kHz is the worst admitted rate for this measurement and 192 kHz the mildest.
RATES = (8000, 22050, 44100, 48000, 96000, 192000)


class TempoMap:
    """Tick -> frame, integrating step and linear-ramp tempo changes.

    A ramp is linear in *tick* space toward the next change's bpm, so the frame
    integral over a ramp segment is K*(t1-t0)/(b1-b0) * ln(b1/b0); a step segment is
    K*(t-t0)/b0. K = 60*SR/PPQ.
    """

    def __init__(self, default_bpm, changes, sr):
        self.k = 60.0 * sr / PPQ
        pts = sorted(({"tick": c["tick"], "bpm": c["bpm"], "ramp": c.get("ramp", False)}
                      for c in changes), key=lambda c: c["tick"])
        if not pts or pts[0]["tick"] > 0:
            pts.insert(0, {"tick": 0, "bpm": default_bpm, "ramp": False})
        self.seg = []                        # (t0, t1|None, b0, b1|None, frames_at_t0)
        acc = 0.0
        for i, p in enumerate(pts):
            nxt = pts[i + 1] if i + 1 < len(pts) else None
            t1 = nxt["tick"] if nxt else None
            b1 = nxt["bpm"] if (nxt and p["ramp"]) else None
            self.seg.append((p["tick"], t1, p["bpm"], b1, acc))
            if t1 is not None:
                acc += self._span(p["tick"], t1, p["tick"], t1, p["bpm"], b1)
        self.seg_starts = [s[0] for s in self.seg]

    def _span(self, t0, t, seg_t0, seg_t1, b0, b1):
        if b1 is None or b1 == b0 or seg_t1 is None:
            return self.k * (t - t0) / b0
        # linear ramp in tick space
        slope = (b1 - b0) / (seg_t1 - seg_t0)
        b_at = b0 + slope * (t - seg_t0)
        return self.k * math.log(b_at / b0) / slope

    def frame(self, tick):
        lo, hi = 0, len(self.seg) - 1
        while lo < hi:                        # last segment with start <= tick
            mid = (lo + hi + 1) // 2
            if self.seg_starts[mid] <= tick:
                lo = mid
            else:
                hi = mid - 1
        t0, t1, b0, b1, base = self.seg[lo]
        return base + self._span(t0, tick, t0, t1, b0, b1)


def played_events(song):
    """Ticks of every event the sequencer actually plays.

    Does NOT honour track mute/solo. No corpus project sets either, so this cannot
    affect the reported numbers, but a project that did would be overcounted.

    Honours Pattern.length, length_override, and PlacementLoopMode: a note stored beyond
    the source pattern's length is never played, a clipped placement stops at the source
    boundary, and a repeating placement repeats until the placement ends.
    """
    pats = {p["id"]: p for p in song.get("patterns", [])}
    ons, offs, autos, targets = [], [], [], set()
    for pl in song.get("arrangement", []):
        p = pats.get(pl["pattern_id"])
        if not p:
            continue
        src = p.get("length") or 0
        if src <= 0:
            continue
        eff = pl.get("length_override") or src
        repeat = (pl.get("loop_mode", "repeat") == "repeat")
        reps = math.ceil(eff / src) if repeat else 1
        base = pl.get("start", 0)
        for r in range(reps):
            off = r * src
            if off >= eff:
                break
            for nt in p.get("notes", []):
                st = nt.get("start", 0)
                if st >= src or off + st >= eff:       # hidden, or past the placement
                    continue
                on = base + off + st
                ons.append(on)
                offs.append(on + nt.get("duration", 0))
            for lane in (p.get("automation") or []):
                targets.add(json.dumps(lane.get("target"), sort_keys=True))
                for pt in lane.get("points", []):
                    t = pt["tick"]
                    if t >= src or off + t >= eff:
                        continue
                    autos.append(base + off + t)
    return ons, offs, autos, targets


def peak_polyphony(ons, offs):
    edges = [(t, 1) for t in ons] + [(t, -1) for t in offs]
    edges.sort(key=lambda e: (e[0], e[1]))    # releases before starts at equal ticks
    cur = peak = 0
    for _, delta in edges:
        cur += delta
        peak = max(peak, cur)
    return peak


paths = sorted(glob.glob("assets/examples/projects/*.ptz")) + \
        sorted(glob.glob("corpus/v2-reference/projects/*.ptz"))
scanned = len(paths)
expansion, empty, rows, measured_inputs = [], [], [], []

for path in paths:
    name = os.path.basename(path)
    try:
        with open(path, encoding="utf-8") as project_file:
            d = json.load(project_file)
    except Exception as exc:                                    # noqa: BLE001
        print(f"SKIP {name}: {exc}")
        continue
    s = d.get("song") or {}
    racks = sum(1 for p in s.get("patterns", []) if p.get("processors"))
    # Count the assets. `next_note_graph_id` is a stable-ID counter, not a count:
    # the Chrome project holds three graphs behind a next-id of four.
    graphs = len(s.get("note_graphs") or [])
    ons, offs, autos, targets = played_events(s)
    if not (ons or autos):
        empty.append(name)
        continue
    if racks or graphs:
        expansion.append((name, racks, graphs))
        continue                                    # cannot be measured without running expansion
    per_rate = {}
    for sr in RATES:
        tm = TempoMap(s.get("default_tempo") or 120.0, s.get("tempo_changes") or [], sr)
        occ = Counter(math.floor(tm.frame(t) + 0.5) // Q for t in ons + offs + autos)
        rel = Counter(math.floor(tm.frame(t) + 0.5) // Q for t in offs)
        per_rate[sr] = (max(occ.values()), max(rel.values()) if rel else 0)
    rows.append((max(v[0] for v in per_rate.values()), name[:40], len(ons), len(autos),
                 len(targets), peak_polyphony(ons, offs), per_rate))
    # The anchor sweep must use exactly the projects and decoded inputs admitted by
    # this pass. Re-reading and re-filtering created a second, divergent population.
    measured_inputs.append((name[:40], s, ons, offs, autos))

if not rows:
    raise SystemExit(
        f"NO MEASURED PROJECTS: scanned {scanned}, found {len(empty)} with no played events "
        f"and excluded {len(expansion)} for note expansion"
    )

rows.sort(reverse=True)
print(f"{'worst':>5} {'project':<40} {'notes':>6} {'autopts':>8} {'lanes':>5} {'poly':>5}   "
      + " ".join(f"{sr//1000:>5}k" for sr in RATES))
for worst, name, nn, na, nl, poly, per in rows:
    cells = " ".join(f"{per[sr][0]:6}" for sr in RATES)
    print(f"{worst:5} {name:<40} {nn:6} {na:8} {nl:5} {poly:5}   {cells}")

print(f"\nscanned {scanned} projects: {len(rows)} measured, {len(empty)} with no played events, "
      f"{len(expansion)} excluded for note expansion")
for n, r, g in expansion:
    print(f"  excluded: {n}  ({r} processor racks, {g} note graphs)")
for n in empty:
    print(f"  no played events: {n}")

worst_occ = max(r[0] for r in rows)
worst_rel = max(max(v[1] for v in r[6].values()) for r in rows)
worst_poly = max(r[5] for r in rows)
print(f"\nworst occupancy over all rates = {worst_occ}  (cap {CAP})")
print(f"worst releases in one quantum  = {worst_rel}")
print(f"peak polyphony                 = {worst_poly}  (max_active_voices = 512)")

# --- anchor-phase sweep -------------------------------------------------------
# ADR-0032 clause 27: a compiled event's SampleTime is time = anchor.time +
# (position - anchor.position), and anchor.time is an integer frame count. The anchor
# therefore shifts every event by a whole number of frames, and the complete phase space
# is the Q integer offsets within one quantum. Prepared-profile plan admission must hold
# for all of them before an anchor exists, so this reports the worst.
print("\nanchor-phase sweep at 8 kHz (worst admitted rate): fixed phase vs worst phase")
print(f"{'fixed':>6} {'worst':>6} {'delta':>6}  project")
# Control FIRST, per plans/v2/evidence/README.md: "that control is run before the
# measurement it guards". If the sweep cannot see a phase difference, every number below
# would be an artifact that looks exactly like a result.
_ctrl = [60, 70, 130]
_c0 = max(Counter(f // Q for f in _ctrl).values())
_cw = max(max(Counter((f + k) // Q for f in _ctrl).values()) for k in range(Q))
print(f"detection control (run first): frames {_ctrl} -> fixed {_c0}, worst {_cw}")
if _cw <= _c0:
    raise SystemExit("CONTROL FAILED: the phase sweep is blind; no anchor-phase result collected")

anchor_rows = []
for name, s, ons, offs, autos in measured_inputs:
    tm = TempoMap(s.get("default_tempo") or 120.0, s.get("tempo_changes") or [], 8000)
    frames = [tm.frame(t) for t in ons + offs + autos]
    # ADR-0032 clause 15: a tempo map creates a PlanPosition by round-half-away-from-zero,
    # and no later stage re-rounds. Flooring here would misbucket an event whose frame lands
    # just under a quantum boundary.
    pos = [math.floor(f + 0.5) for f in frames]
    fixed = max(Counter(x // Q for x in pos).values())
    worst = max(max(Counter((x + k) // Q for x in pos).values()) for k in range(Q))
    anchor_rows.append((worst, fixed, name))
if not anchor_rows:
    raise SystemExit("NO ANCHOR RESULTS: the measured-project population was unexpectedly empty")
anchor_rows.sort(reverse=True)
for w, f, n in anchor_rows:
    print(f"{f:6} {w:6} {w-f:+6}  {n}")
print(f"worst over every anchor phase = {max(r[0] for r in anchor_rows)}  (cap {CAP})")


print("\ntick instants one quantum can intersect, worst phase, per rate at 93 and 200 BPM:")
print(f"{'rate':>8} " + " ".join(f"{b:>10}" for b in (93, 200)))
for sr in RATES:
    cells = []
    for bpm in (93, 200):
        fpt = (60.0 / bpm) * sr / PPQ
        worst = 0
        for step in range(4000):
            start = step * 0.25
            worst = max(worst, math.ceil((start + Q) / fpt) - math.ceil(start / fpt))
        # Report only the onset contribution from expansion performed at tick instants
        # inside this quantum. Releases produced earlier and simultaneous placements can
        # add more, so this is deliberately not labelled a total-occupancy upper bound.
        cells.append(f"{worst:4} ({worst*128:5})")
    print(f"{sr:8} " + " ".join(f"{c:>10}" for c in cells))
print("\n(instants, and current-quantum onset contribution at max_note_expansion_per_tick = 128 each)")
