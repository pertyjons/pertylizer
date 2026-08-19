"""Apply EVD-0012's rule table to the raw sweep, mechanically.

Ratios are formed WITHIN a sweep from that sweep's own figures, then the median over
sweeps is reported: a machine that drifts over the run drifts through both members of
each pair together, which a ratio of two separately pooled figures would absorb instead
of cancelling.
"""
import csv, statistics, sys
from collections import defaultdict

# The sweep CSV to read: the retained artifact beside this file by default, or a fresh
# collection named on the command line.
source = sys.argv[1] if len(sys.argv) > 1 else (
    __file__.rsplit('/', 1)[0] + '/EVD-0012-render-quantum-real-path.csv')
rows = list(csv.DictReader(open(source)))
# cost[(sweep, shape, variant, arm)] = ms per rendered second
cost, spread = {}, defaultdict(list)
for r in rows:
    key = (int(r['sweep']), r['shape'], r['variant'], r['arm'])
    cost[key] = float(r['cost_ms_per_s'])
    spread[(r['shape'], r['variant'])].append(float(r['in_process_spread_percent']))

sweeps = sorted({int(r['sweep']) for r in rows})
shapes = ['voice-mono', 'voice-stereo', 'gain-chain']
variants = ['as-built', 'clause-5']

def ratio(sweep, shape, variant, a, b):
    return (cost[(sweep, shape, variant, a)] / cost[(sweep, shape, variant, b)] - 1) * 100

PAIRS = {'r(64,256)': ('64', '256'), 'r(128,256)': ('128', '256'),
         'r(32,64)': ('32', '64'), 'null r(64b,64)': ('64b', '64')}

print(f"{'shape':<13} {'variant':<9} " + " ".join(f"{n:>15}" for n in PAIRS) + f" {'N':>7} {'spread':>7}")
results = {}
for shape in shapes:
    for variant in variants:
        med = {}
        for name, (a, b) in PAIRS.items():
            med[name] = statistics.median(ratio(s, shape, variant, a, b) for s in sweeps)
        null = statistics.median(abs(ratio(s, shape, variant, '64b', '64')) for s in sweeps)
        # Per-comparison dispersion: the same binaries have no true sweep-to-sweep
        # variation, so a ratio's spread across sweeps is what the instrument does to it.
        def mad(name):
            values = [ratio(s, shape, variant, *PAIRS[name]) for s in sweeps]
            centre = statistics.median(values)
            return statistics.median(abs(v - centre) for v in values)
        noise = {n: max(null, mad(n)) for n in PAIRS}
        insp = statistics.median(spread[(shape, variant)])
        results[(shape, variant)] = (med, noise, null)
        print(f"{shape:<13} {variant:<9} " + " ".join(f"{med[n]:>+14.2f}%" for n in PAIRS)
              + f" {null:>6.2f}% {insp:>6.2f}%")

print()
print("Rule evaluation, in ADR-0037's order, with EVD-0012 rule 1' first:")
print(f"{'shape':<13} {'variant':<9} {'rule':<8} {'outcome':<24} margins")
outcomes = {}
for shape in shapes:
    for variant in variants:
        med, noise, null = results[(shape, variant)]
        reached = [('r(64,256)', med['r(64,256)'], 15.0)]
        if med['r(64,256)'] > 15.0:
            reached.append(('r(128,256)', med['r(128,256)'], 15.0))
            rule, outcome = ('2', 'select 128') if med['r(128,256)'] <= 15.0 else ('3', 'escalate: all expensive')
        else:
            reached.append(('r(32,64)', med['r(32,64)'], 2.0))
            rule, outcome = ('4', 'select 32') if med['r(32,64)'] <= 2.0 else ('5', 'confirm 64')
        margins = [(n, abs(v - t), noise[n]) for n, v, t in reached]
        unresolvable = [(n, m) for n, m, f in margins if m < f]
        if unresolvable:
            rule, outcome = "1'(a)", 'unresolvable: below noise'
        outcomes[(shape, variant)] = (rule, outcome)
        text = " ".join(f"{n} margin {m:.2f}pp vs N {f:.2f}pp" for n, m, f in margins)
        print(f"{shape:<13} {variant:<9} {rule:<8} {outcome:<24} {text}")

print()
gov = {v: outcomes[('voice-mono', v)][0] for v in variants}
print(f"Governing shape voice-mono: as-built rule {gov['as-built']}, clause-5 rule {gov['clause-5']}")
if gov['as-built'] != gov['clause-5']:
    print("=> rule 1'(b) FIRES: the variants select different rules")
disagree = [(s, v) for s in shapes[1:] for v in variants
            if outcomes[(s, v)][0] != outcomes[('voice-mono', v)][0]]
if disagree:
    print(f"=> rule 1'(c) FIRES: bounding shapes disagree: {disagree}")
if not disagree and gov['as-built'] == gov['clause-5']:
    print(f"=> rule {gov['as-built']} stands: {outcomes[('voice-mono','as-built')][1]}")
