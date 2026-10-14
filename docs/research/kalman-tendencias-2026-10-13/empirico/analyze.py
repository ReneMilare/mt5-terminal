#!/usr/bin/env python3
"""Descriptive aggregation only; no profit, significance tests or optimized scalar score."""
import csv
from collections import defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
RESULTS = HERE / 'results'

def load(path):
    with path.open() as f:
        return list(csv.DictReader(f))

def num(row, col):
    return float(row[col])

def divide(a, b):
    return a/b if b else float('nan')

def aggregate(rows):
    keys = ('kernel', 'entry', 'exit', 'view', 'horizon_min')
    groups = defaultdict(list)
    for row in rows:
        for split in (row['split'], 'all_reused'):
            groups[tuple(row[k] for k in keys)+(split,)].append(row)
    output = []
    for key, group in sorted(groups.items()):
        out = dict(zip(keys+('split',), key))
        days = sum(num(r, 'days') for r in group)
        bars = sum(num(r, 'rth_bars') for r in group)
        eligible = sum(num(r, 'labelled') for r in group)
        colored = sum(num(r, 'colored') for r in group)
        correct = sum(num(r, 'correct') for r in group)
        reversals = sum(num(r, 'reversals') for r in group)
        out.update(symbol_sessions=int(days),rth_bars=int(bars),labelled=int(eligible),
                   direction_accuracy=divide(correct,colored),correct_capture=divide(correct,eligible),
                   reversal_miss_fraction=divide(sum(num(r,'reversal_misses') for r in group),reversals))
        for col, numerator, denominator in [('occupancy','state_colored','rth_bars'),
            ('magnitude_capture','magnitude_correct','magnitude_total'),
            ('opposite_magnitude_capture','magnitude_wrong','magnitude_total'),
            ('opposite_slope_fraction','opposite_slope_count','state_colored'),
            ('past_direction_accuracy','past_correct','past_colored'),
            ('past_correct_capture','past_correct','past_labelled')]:
            out[col]=divide(sum(num(r,numerator) for r in group),sum(num(r,denominator) for r in group))
        for col in ('changes_per_session','flips_per_session'):
            out[col] = divide(sum(num(r,col)*num(r,'days') for r in group),days)
        # These are explicitly means of symbol/block medians, not pooled medians.
        for col in ('median_run_min','level_median_atr','level_p90_atr','speed_p10','speed_median','speed_p90','reversal_lag_median_min','reversal_lag_p90_min'):
            out['mean_'+col] = sum(num(r,col) for r in group)/len(group)
        output.append(out)
    return output

def write(path,rows):
    with path.open('w') as f:
        w=csv.DictWriter(f,fieldnames=list(rows[0]));w.writeheader();w.writerows(rows)

def old_visual():
    base=HERE.parents[3]/'preset/tools/research/kalman-trend-m5-h1-2026-10-13/results'
    params=load(base/'parameters.csv')
    names={r['state']:r['name'] for r in params}
    groups=defaultdict(list)
    for row in load(base/'visual.csv'):
        groups[(row['state'],row['entry'],row['exit'])].append(row)
    out=[]
    for (state,entry,exit),group in groups.items():
        days=sum(num(r,'days') for r in group);bars=sum(num(r,'rth_bars') for r in group)
        out.append(dict(kernel=names[state],state=state,entry=entry,exit=exit,
                        changes_per_session=divide(sum(num(r,'changes') for r in group),days),
                        occupancy=divide(sum(num(r,'colored') for r in group),bars),
                        aligned_occupancy=divide(sum(num(r,'aligned') for r in group),bars),
                        mean_median_minutes=sum(num(r,'median_minutes') for r in group)/len(group)))
    out.sort(key=lambda r:(r['changes_per_session'],-r['occupancy']))
    write(RESULTS/'existing_visual_descriptive.csv',out)
    return out

def main():
    rows=load(RESULTS/'metrics.csv');agg=aggregate(rows);write(RESULTS/'aggregated.csv',agg)
    old=old_visual()
    lines=['All results reuse history; aggregation combines both symbols. Medians below are means of group medians.',
           'Horizon 60 min, all reused blocks; illustrative predefined controls, not an optimized score:',
           'kernel entry exit view accuracy capture occupancy flips/session meanRunMin reversalMiss meanLag levelMedATR']
    for r in agg:
        if r['horizon_min']!='60' or r['split']!='all_reused':continue
        if (r['entry'],r['exit']) not in [('1','0'),('0.16','-0.92'),('0.1','0.1')]:continue
        lines.append(f"{r['kernel']:24} {r['entry']:4} {r['exit']:5} {r['view']:12} {r['direction_accuracy']:.3f} {r['correct_capture']:.3f} {r['occupancy']:.3f} {r['flips_per_session']:.2f} {r['mean_median_run_min']:.1f} {r['reversal_miss_fraction']:.3f} {r['mean_reversal_lag_median_min']:.1f} {r['mean_level_median_atr']:.3f}")
    lines.append('\nOld visual CSV: ten most stable configurations by changes/session (descriptive only)')
    for r in old[:10]:lines.append(str(r))
    (RESULTS/'summary.txt').write_text('\n'.join(lines)+'\n')
    print('\n'.join(lines))

if __name__=='__main__': main()
