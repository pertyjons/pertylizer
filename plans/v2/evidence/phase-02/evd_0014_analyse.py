#!/usr/bin/env python3
"""EVD-0014's rule table, applied mechanically rather than by eye.

    python3 plans/v2/evidence/phase-02/evd_0014_analyse.py <null.csv>
    python3 plans/v2/evidence/phase-02/evd_0014_analyse.py <null.csv> <sweeps.csv>

Both files come from `crates/pertylizer/examples/evd_0014_cost.rs`. **With one
argument it evaluates the null collection and stops**, which is what makes rule 0
executable in the order the method states: the floor is read before the
comparison sweeps are collected, and a floor above `N_max` means the instrument
needs attention rather than the engines.

Every figure is formed the way the record states, and nowhere else:

- a slot's figure for a (pair, variant) is already the minimum over rounds;
- the sweep's engine figure is the **mean of that engine's two slots**, which is
  what balances slot against position;
- the comparison ratio is formed **within a sweep**, from that sweep's own
  figures, and the reported `r` is the **median over sweeps** of those paired
  ratios;
- the noise floor `N` is the **largest** of three quantities: the null
  collection's median absolute ratio, the comparison's own median absolute
  deviation across sweeps, and the comparison's own **within-sweep null ratios**
  — `c(V1a)/c(V1b) - 1` and `c(V2a)/c(V2b) - 1`, each of which is two
  measurements of one engine and therefore has a true value of zero.

The third was added after collection and before any verdict was read, because
the null pass holds one engine in all four slots and so never measures V1's own
variability. Taking the largest can only make a comparison **harder** to
resolve, which is the direction a correction to an acceptance rule has to run
in.

Malformed evidence is refused rather than silently averaged: a collection
missing sweeps, slots, pairs or variants, or carrying a duplicate row, produces
an error instead of a verdict.
"""

import csv
import statistics
import sys

# Rule 0's ceiling, fixed before collection. EVD-0010, EVD-0011 and EVD-0012 all
# resolved margins on this machine with floors between roughly 0.9 and 4.6
# percentage points; a floor above this would put the collection outside
# everything previously measured here.
N_MAX = 0.05

# The pair the gate is decided on. The other is context and cannot fail it.
GOVERNING = "voice-dsp"

# The variant the gate is evaluated on: clause 5's input carry is a surcharge on
# V2 alone, so omitting it can only flatter V2.
GATE_VARIANT = "clause-5"


def read(path):
    """{(sweep, pair, variant, slot): cost}.

    A duplicate key is an error rather than an overwrite: two rows for one
    measurement mean the collection is not what it says it is, and silently
    keeping the last would hide that.
    """
    rows = {}
    with open(path, newline="") as handle:
        for row in csv.DictReader(handle):
            key = (int(row["sweep"]), row["pair"], row["variant"], row["slot"])
            if key in rows:
                raise SystemExit(f"{path}: duplicate row for {key}")
            rows[key] = float(row["cost_ms_per_s"])
    if not rows:
        raise SystemExit(f"{path}: no rows")
    return rows


def check_coverage(path, rows, slots):
    """Refuse a collection that is not the declared shape.

    Without this, a truncated file still produces a verdict — from however many
    sweeps survived — and the verdict looks exactly like one from a complete
    collection.
    """
    sweeps = sorted({key[0] for key in rows})
    if len(sweeps) % 24 != 0:
        raise SystemExit(
            f"{path}: {len(sweeps)} sweeps, which is not a multiple of the 24 permutations "
            "the method declares"
        )
    if sweeps != list(range(len(sweeps))):
        raise SystemExit(f"{path}: sweep indices are not contiguous from zero")
    missing = []
    for sweep in sweeps:
        for pair, variant in (
            ("voice-dsp", "as-built"),
            ("voice-dsp", "clause-5"),
            ("whole-render", "as-built"),
        ):
            for slot in slots:
                if slot.startswith("V1") and variant == "clause-5":
                    continue  # V1 has no counterpart to clause 5's input carry.
                if (sweep, pair, variant, slot) not in rows:
                    missing.append((sweep, pair, variant, slot))
    if missing:
        raise SystemExit(
            f"{path}: {len(missing)} measurements missing, first {missing[:3]}"
        )
    return len(sweeps)


def engine_figure(rows, sweep, pair, variant, slots):
    """The mean of an engine's two slots for one sweep, or None if absent.

    V1 has no `clause-5` arm, so a V1 figure for that variant falls back to its
    `as-built` one — which is the point of the variant: it is a surcharge on V2
    measured against an unchanged V1.
    """
    values = []
    for slot in slots:
        value = rows.get((sweep, pair, variant, slot))
        if value is None and slot.startswith("V1"):
            value = rows.get((sweep, pair, "as-built", slot))
        if value is None:
            return None
        values.append(value)
    return sum(values) / len(values)


def ratios(rows, pair, variant, reference_slots, candidate_slots):
    """Paired within-sweep ratios, one per sweep."""
    out = []
    for sweep in sorted({key[0] for key in rows}):
        reference = engine_figure(rows, sweep, pair, variant, reference_slots)
        candidate = engine_figure(rows, sweep, pair, variant, candidate_slots)
        if reference in (None, 0.0) or candidate is None:
            continue
        out.append(candidate / reference - 1.0)
    return out


def within_sweep_nulls(rows):
    """The two null ratios the comparison collection carries in itself.

    Each pairs an engine's two slots, so its true value is zero. This is what the
    null pass cannot supply: it holds one engine in all four slots, so it never
    measures the other engine's variability.
    """
    out = {}
    for engine in ("V1", "V2"):
        for pair in ("voice-dsp", "whole-render"):
            for variant in ("as-built", "clause-5"):
                values = []
                for sweep in sorted({key[0] for key in rows}):
                    a = rows.get((sweep, pair, variant, f"{engine}a"))
                    b = rows.get((sweep, pair, variant, f"{engine}b"))
                    if a and b:
                        values.append(a / b - 1.0)
                if values:
                    out[(engine, pair, variant)] = statistics.median(
                        [abs(v) for v in values]
                    )
    return out


def mad(values):
    """Median absolute deviation about the median."""
    if not values:
        return float("inf")
    centre = statistics.median(values)
    return statistics.median([abs(v - centre) for v in values])


def rule(r, n):
    """The record's rule table, in order, stopping at the first that applies."""
    if n > N_MAX:
        return 0, "Inconclusive — the instrument cannot resolve this comparison"
    if r + n < 0:
        return 1, "Pass — V2 is measurably cheaper"
    if abs(r) <= n:
        return 2, "Pass, not separable"
    return 3, "Pass with a documented margin, or rule 4 — see the decomposition"


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        raise SystemExit(2)
    null_rows = read(sys.argv[1])
    check_coverage(sys.argv[1], null_rows, ["V2a", "V2b", "V2c", "V2d"])

    print("# EVD-0014 rule table")
    print(f"n_max,{N_MAX}")
    print(f"governing_pair,{GOVERNING}")
    print(f"gate_variant,{GATE_VARIANT}")
    print()

    # --- C1: the null pass. Four slots of one engine, paired the same way. ---
    print("# C1 null pass: every ratio has a true value of zero")
    print("pair,variant,sweeps,median_r,median_abs_r,mad")
    floors = {}
    worst_null = 0.0
    for pair in ("voice-dsp", "whole-render"):
        for variant in ("as-built", "clause-5"):
            values = ratios(null_rows, pair, variant, ["V2a", "V2b"], ["V2c", "V2d"])
            if not values:
                continue
            floor = statistics.median([abs(v) for v in values])
            floors[(pair, variant)] = floor
            worst_null = max(worst_null, floor)
            print(
                f"{pair},{variant},{len(values)},{statistics.median(values):+.6f},"
                f"{floor:.6f},{mad(values):.6f}"
            )

    # Rule 0, executable in the order the method states: the floor decides
    # whether the comparison is worth collecting at all.
    print()
    if worst_null > N_MAX:
        print(f"rule_0,fires,{worst_null:.6f},>,{N_MAX}")
        print("outcome,Inconclusive — the instrument cannot resolve this comparison")
        print("# The comparison sweeps are not run. The instrument needs attention.")
        return
    print(f"rule_0,does_not_fire,{worst_null:.6f},<=,{N_MAX}")

    if len(sys.argv) < 3:
        print("# Null collection only. Pass the comparison CSV to evaluate the gate.")
        return

    sweep_rows = read(sys.argv[2])
    check_coverage(sys.argv[2], sweep_rows, ["V1a", "V1b", "V2a", "V2b"])
    nulls = within_sweep_nulls(sweep_rows)

    print()
    print("# The comparison's own within-sweep nulls, which the null pass cannot supply")
    print("engine,pair,variant,median_abs_r")
    for key in sorted(nulls):
        print(f"{key[0]},{key[1]},{key[2]},{nulls[key]:.6f}")

    print()
    print("# The comparison, V2 against V1")
    print("pair,variant,sweeps,r,null_floor,own_mad,v1_null,v2_null,N,rule,outcome")
    verdicts = {}
    for pair in ("voice-dsp", "whole-render"):
        for variant in ("as-built", "clause-5"):
            values = ratios(sweep_rows, pair, variant, ["V1a", "V1b"], ["V2a", "V2b"])
            if not values:
                continue
            r = statistics.median(values)
            null_floor = floors.get((pair, variant), 0.0)
            own = mad(values)
            # V1 has no clause-5 arm, so its within-sweep null for that variant
            # is the as-built one — the same measurements under another label.
            v1_null = nulls.get(("V1", pair, variant), nulls.get(("V1", pair, "as-built"), 0.0))
            v2_null = nulls.get(("V2", pair, variant), 0.0)
            n = max(null_floor, own, v1_null, v2_null)
            number, outcome = rule(r, n)
            verdicts[(pair, variant)] = (r, n, number, outcome)
            print(
                f"{pair},{variant},{len(values)},{r:+.6f},{null_floor:.6f},"
                f"{own:.6f},{v1_null:.6f},{v2_null:.6f},{n:.6f},{number},{outcome}"
            )

    print()
    key = (GOVERNING, GATE_VARIANT)
    if key in verdicts:
        r, n, number, outcome = verdicts[key]
        print("# The gate, decided on the governing pair and the conservative variant")
        print(f"gate_r,{r:+.6f}")
        print(f"gate_N,{n:.6f}")
        print(f"gate_rule,{number}")
        print(f"gate_outcome,{outcome}")
        margin = abs(r) / n if n > 0 else float("inf")
        print(f"gate_margin_over_floor,{margin:.2f}")


if __name__ == "__main__":
    main()
