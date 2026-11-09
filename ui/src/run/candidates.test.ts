import { describe, expect, it } from 'vitest';
import type { Event, EventKind } from '../types';
import { objectiveText, studyOf, ticks } from './candidates';

let seq = 0;
const ev = (kind: EventKind, payload: unknown): Event => ({ branch_id: 'b', seq: ++seq, at: '', agent: 'optimiser', kind, payload });

const sim = (n: number, value: number, over: Record<string, unknown> = {}) => ev('tool_call', {
  name: 'run_simulation', args: {}, error: null, reads: [], writes: [], snapshot: null, tool_call_id: null,
  result: { simulation: n, objective: { name: 'line2.oee', label: 'Line 2 OEE', value, ci95: 0.8, unit: '%', better: 'higher' }, feasible: true, changes: {}, ...over },
});

describe('an optimisation run’s candidates', () => {
  const events = [
    ev('tool_call', { name: 'describe_model', args: {}, error: null, result: { objective_kpi: 'line2.oee' } }),
    sim(1, 76.9),
    sim(2, 82.1, { changes: { 'line2.batch_size': 240 } }),
    sim(3, 88.0, { feasible: false, changes: { 'line2.batch_size': 400 } }),
    ev('tool_call', { name: 'run_simulation', args: {}, error: '{"denied": true}', result: null }),
    sim(4, 85.9, { changes: { 'line2.crew': [2, 2, 1, 1] } }),
    sim(5, 84.0),
    ev('check', { passed: true, gold_db: null, differing_rows: [], checks: [{ name: 'goal_reached', passed: true, measure: { label: 'Line 2 OEE', value: 86.2, target: 85 } }] }),
  ];
  const s = studyOf(events)!;

  it('finds each candidate with its number, value, interval, feasibility and changes', () => {
    expect(s.objective).toEqual({ label: 'Line 2 OEE', unit: '%', better: 'higher' });
    expect(s.candidates.map((c) => c.n)).toEqual([1, 2, 3, 4, 5]);
    expect(s.candidates[1]).toMatchObject({ value: 82.1, ci: 0.8, feasible: true, changes: 'line2.batch_size 240' });
    expect(s.candidates[3].changes).toBe('line2.crew 2/2/1/1');
  });

  it('keeps the best feasible value so far, skipping a candidate that missed a limit', () => {
    expect(s.candidates.map((c) => c.best)).toEqual([76.9, 82.1, 82.1, 85.9, 85.9]);
  });

  it('takes the target and the verified value from the check with the same label', () => {
    expect([s.target, s.verified]).toEqual([85, 86.2]);
  });

  it('is null for a run that evaluated no candidate', () => {
    expect(studyOf([ev('tool_call', { name: 'get_inventory', args: {}, error: null, result: { items: [] } })])).toBeNull();
  });

  it('a lower-is-better objective keeps the smallest', () => {
    const low = (n: number, v: number) => sim(n, v, { objective: { label: 'Lead time', value: v, unit: 'h', better: 'lower' } });
    expect(studyOf([low(1, 5), low(2, 3), low(3, 4)])!.candidates.map((c) => c.best)).toEqual([5, 3, 3]);
  });

  it('labels and ticks the axis in round steps', () => {
    expect(ticks(76.3, 89.1)).toEqual([80, 85]);
    expect(ticks(0, 30, 6)).toEqual([0, 5, 10, 15, 20, 25, 30]);
    expect(objectiveText(86.44, '%')).toBe('86.4%');
    expect(objectiveText(3.2, 'h')).toBe('3.2 h');
  });
});
