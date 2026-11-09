import { describe, expect, it } from 'vitest';
import type { ListedExperiment } from '../experiments/types';
import type { Branch, RunSummary } from '../types';
import { hereOf, itemsFor, placeholderOf } from './palette';
import { nextPref, parsePref, resolveTheme } from './themeModel';

const branch = (id: string, status: Branch['status'], parent: string | null = null, at = '2026-11-06T10:00:00Z'): Branch =>
  ({ id, run_id: 'r1', parent_id: parent, start_seq: parent ? 5 : null, team: { id: 't', name: 't', agents: [] }, changes: null, fork_note: null, status, error: null, created_at: at, ended_at: null });
const runs: RunSummary[] = [{ id: 'r1', name: 'Demo run', task_id: 'cvx', created_at: '2026-11-06T10:00:00Z', branches: [branch('b1', 'passed'), branch('b2', 'running', 'b1', '2026-11-06T11:00:00Z')] }];
const experiment = (id: string, name: string): ListedExperiment => ({
  id, name, tasks: ['a'], benchmark: 'b', runs: 5, created_at: '', status: 'finished', legacy: false,
  setups: [{ suite_id: `${id}-a`, name: 'Basic', team_name: 'T', runner: 'chain', runs: 5, ended: 5, passed: 3 }],
});
const experiments = [experiment('e1', 'Rules or not'), experiment('e2', 'Haiku vs Sonnet')];

describe('command palette', () => {
  it('lists pages, actions, every experiment and every branch at the root', () => {
    const items = itemsFor({ kind: 'root' }, runs, experiments);
    expect(items.filter((i) => i.group === 'Go to').map((i) => i.label)).toEqual(['Experiments', 'Runs', 'Team editor']);
    expect(items.find((i) => i.id === 'new-experiment')!.action).toEqual({ go: '/experiments/new' });
    expect(items.find((i) => i.id === 'new-run')!.action).toEqual({ go: '/run' });
    expect(items.filter((i) => i.group === 'Runs').map((i) => [i.label, i.action])).toEqual([
      ['Demo run', { go: '/runs/r1/b1' }], ['Demo run · fork b2', { go: '/runs/r1/b2' }],
    ]);
    expect(items.find((i) => i.id === 'experiment-e2')).toMatchObject({ label: 'Haiku vs Sonnet', hint: '1 setup · finished', action: { go: '/experiments/e2' } });
  });
  it('compares two runs in two picks, finished branches only', () => {
    const first = itemsFor({ kind: 'compare-runs', first: null }, [{ ...runs[0], branches: [...runs[0].branches, branch('b3', 'failed')] }], experiments);
    expect(first.map((i) => i.id)).toEqual(['branch-b1', 'branch-b3']);
    expect(first[0].action).toEqual({ mode: { kind: 'compare-runs', first: 'b1' } });
    const second = itemsFor({ kind: 'compare-runs', first: 'b1' }, [{ ...runs[0], branches: [...runs[0].branches, branch('b3', 'failed')] }], experiments);
    expect(second.map((i) => i.action)).toEqual([{ go: '/compare/b1/b3' }]);
    expect(placeholderOf({ kind: 'compare-runs', first: 'b1' })).toBe('Pick the run to compare it with…');
  });
  it('opens any run branch\'s world view, and the current run\'s first', () => {
    expect(itemsFor({ kind: 'world' }, runs, experiments).map((i) => i.action)).toEqual([{ go: '/world/r1/b1' }, { go: '/world/r1/b2' }]);
    expect(itemsFor({ kind: 'root' }, runs, experiments).find((i) => i.id === 'world')!.action).toEqual({ mode: { kind: 'world' } });
    const here = hereOf('/runs/r1/b2');
    expect(here).toEqual({ page: 'runs', runId: 'r1', branchId: 'b2' });
    expect(itemsFor({ kind: 'root' }, runs, experiments, here)[0]).toMatchObject({ id: 'world-here', action: { go: '/world/r1/b2' } });
    expect(itemsFor({ kind: 'root' }, runs, experiments, hereOf('/world/r1/b2')).map((i) => i.id).slice(0, 2)).toEqual(['runs-here', 'control-here']);
    expect(hereOf('/run', '?run=r1&branch=b2')).toEqual({ page: 'home', runId: 'r1', branchId: 'b2' });
    expect(hereOf('/run', '')).toBeNull();
    expect(hereOf('/experiments/e1')).toBeNull();
    expect(placeholderOf({ kind: 'world' })).toBe('Pick the run whose world to open…');
  });
});

describe('theme', () => {
  it('resolves the preference against the system and cycles through the three', () => {
    expect(resolveTheme('system', true)).toBe('dark');
    expect(resolveTheme('light', true)).toBe('light');
    expect([nextPref('system'), nextPref('light'), nextPref('dark')]).toEqual(['light', 'dark', 'system']);
    expect([parsePref('dark'), parsePref('nonsense'), parsePref(null)]).toEqual(['dark', 'system', 'system']);
  });
});
