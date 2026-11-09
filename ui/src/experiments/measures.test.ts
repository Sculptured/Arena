import { describe, expect, it } from 'vitest';
import type { RunOutcome, RunRow, SuiteReport } from '../control/types';
import type { NamedCheck } from '../types';
import { measureRows, measureText, position } from './measures';

const OEE = { label: 'Line 2 OEE', unit: '%', better: 'higher' as const, target: 85, reference: { label: 'Random search, 30 simulations', value: 77.2 } };

const checks = (oee: number | null, sims: number): NamedCheck[] => [
  { name: 'recommendation_submitted', passed: oee != null },
  oee == null ? { name: 'goal_reached', passed: false } : { name: 'goal_reached', passed: oee >= 85, measure: { ...OEE, value: oee } },
  { name: 'within_budget', passed: true, measure: { label: 'Simulations', value: sims } },
];

const outcome = (oee: number | null, sims: number, scored = true): RunOutcome => ({
  status: 'passed', error: null, checks: checks(oee, sims), scored, checks_passed: 1, checks_total: 2, safety_violations: 0,
  denials: 0, flags: 0, measured_s: 1, estimated_s: null, tokens: 1, cost_usd: null, peak_context: null,
});

const row = (i: number, o: RunOutcome | null): RunRow => ({ task_id: 't', repeat: i, run_id: `r${i}`, branch_id: `b${i}`, status: o ? 'passed' : 'running', error: null, outcome: o });

const report = (id: string, rows: RunRow[]) => ({ id, name: id, runs: rows } as unknown as SuiteReport);

describe('measures per setup', () => {
  const a = report('A', [row(1, outcome(87.5, 15)), row(2, outcome(83.1, 22)), row(3, outcome(null, 30)), row(4, null)]);
  const b = report('B', [row(1, outcome(86.0, 12)), row(2, outcome(88.9, 18, false))]);
  const [oee, sims] = measureRows([a, b]);

  it('makes one row per measured check, in the order they first appear', () => {
    expect([oee.name, sims.name]).toEqual(['goal_reached', 'within_budget']);
    expect(oee).toMatchObject({ label: 'Line 2 OEE', unit: '%', better: 'higher', target: 85, reference: OEE.reference });
    expect(sims.better).toBeNull();
  });

  it('headlines the best run when the measure has a better way, else the mean', () => {
    expect(oee.cells.map((c) => c.headline)).toEqual([87.5, 86]);
    expect(oee.cells[0].mean).toBeCloseTo(85.3);
    expect(sims.cells.map((c) => c.headline)).toEqual([(15 + 22 + 30) / 3, 12]);
  });

  it('counts only scored runs that measured it, and links each dot to its run', () => {
    expect(oee.cells[0].dots.map((d) => d.value)).toEqual([87.5, 83.1]);
    expect(oee.cells[1].dots).toHaveLength(1); // the unscored run is left out
    expect(oee.cells[0].dots[0]).toEqual({ value: 87.5, href: '/runs/r1/b1', title: 'Run 1 · 87.5%' });
  });

  it('marks the best setup only for a measure with a better way', () => {
    expect(oee.cells.map((c) => c.best)).toEqual([true, false]);
    expect(sims.cells.every((c) => !c.best)).toBe(true);
  });

  it('puts every setup, the target and the reference on one scale; counts from zero', () => {
    const [lo, hi] = oee.domain;
    expect(lo).toBeLessThan(77.2);
    expect(hi).toBeGreaterThan(87.5);
    expect(sims.domain[0]).toBe(0);
    expect(position(oee, lo)).toBe(0);
    expect(position(oee, hi)).toBe(100);
    expect(position({ domain: [5, 5] }, 5)).toBe(50);
  });

  it('has no rows when no check measures anything', () => {
    expect(measureRows([report('C', [row(1, { ...outcome(80, 1), checks: [{ name: 'x', passed: true }] })])])).toEqual([]);
  });

  it('writes values with their unit', () => {
    expect(measureText(86.43, '%')).toBe('86.4%');
    expect(measureText(17, '')).toBe('17');
    expect(measureText(3.25, 'h')).toBe('3.3 h');
    expect(measureText(null, '%')).toBe('–');
  });
});
