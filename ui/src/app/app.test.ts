import { describe, expect, it } from 'vitest';
import type { ListedSuite } from '../control/types';
import type { Branch, RunSummary } from '../types';
import { hereOf, itemsFor, placeholderOf } from './palette';
import { nextPref, parsePref, resolveTheme } from './themeModel';

const branch = (id: string, status: Branch['status'], parent: string | null = null, at = '2026-11-06T10:00:00Z'): Branch =>
  ({ id, run_id: 'r1', parent_id: parent, start_seq: parent ? 5 : null, team: { id: 't', name: 't', agents: [] }, changes: null, fork_note: null, status, error: null, created_at: at, ended_at: null });
const runs: RunSummary[] = [{ id: 'r1', name: 'Demo run', task_id: 'cvx', created_at: '2026-11-06T10:00:00Z', branches: [branch('b1', 'passed'), branch('b2', 'running', 'b1', '2026-11-06T11:00:00Z')] }];
const suite = (id: string, name: string): ListedSuite => ({ id, name, team: { id: 't', name: 'Team', agents: [] }, tasks: ['a'], repeats: 1, runner: 'chain', benchmark: 'b', created_at: '', ended_at: null, error: null, status: 'finished', runs: 2, ended: 2, passed: 1 });
const suites = [suite('s1', 'v1'), suite('s2', 'v2')];

describe('command palette', () => {
  it('lists pages, actions, every branch and every suite at the root', () => {
    const items = itemsFor({ kind: 'root' }, runs, suites);
    expect(items.filter((i) => i.group === 'Go to').map((i) => i.label)).toEqual(['Run', 'History', 'Suites', 'Team editor']);
    expect(items.find((i) => i.id === 'new-run')!.action).toEqual({ go: '/' });
    expect(items.find((i) => i.id === 'new-suite')!.action).toEqual({ go: '/suites?new=suite' });
    expect(items.filter((i) => i.group === 'Runs').map((i) => [i.label, i.action])).toEqual([
      ['Demo run', { go: '/runs/r1/b1' }], ['Demo run · fork b2', { go: '/runs/r1/b2' }],
    ]);
    expect(items.find((i) => i.id === 'suite-s2')!.action).toEqual({ go: '/suites/s2' });
  });
  it('compares two runs in two picks, finished branches only', () => {
    const first = itemsFor({ kind: 'compare-runs', first: null }, [{ ...runs[0], branches: [...runs[0].branches, branch('b3', 'failed')] }], suites);
    expect(first.map((i) => i.id)).toEqual(['branch-b1', 'branch-b3']);
    expect(first[0].action).toEqual({ mode: { kind: 'compare-runs', first: 'b1' } });
    const second = itemsFor({ kind: 'compare-runs', first: 'b1' }, [{ ...runs[0], branches: [...runs[0].branches, branch('b3', 'failed')] }], suites);
    expect(second.map((i) => i.action)).toEqual([{ go: '/compare/b1/b3' }]);
    expect(placeholderOf({ kind: 'compare-runs', first: 'b1' })).toBe('Pick the run to compare it with…');
  });
  it('compares two suites', () => {
    expect(itemsFor({ kind: 'compare-suites', first: 's1' }, runs, suites).map((i) => i.action)).toEqual([{ go: '/suites/compare/s1/s2' }]);
  });
  it('opens any run branch\'s world view, and the current run\'s first', () => {
    expect(itemsFor({ kind: 'world' }, runs, suites).map((i) => i.action)).toEqual([{ go: '/world/r1/b1' }, { go: '/world/r1/b2' }]);
    expect(itemsFor({ kind: 'root' }, runs, suites).find((i) => i.id === 'world')!.action).toEqual({ mode: { kind: 'world' } });
    const here = hereOf('/runs/r1/b2');
    expect(here).toEqual({ page: 'runs', runId: 'r1', branchId: 'b2' });
    expect(itemsFor({ kind: 'root' }, runs, suites, here)[0]).toMatchObject({ id: 'world-here', action: { go: '/world/r1/b2' } });
    expect(itemsFor({ kind: 'root' }, runs, suites, hereOf('/world/r1/b2')).map((i) => i.id).slice(0, 2)).toEqual(['runs-here', 'control-here']);
    expect(hereOf('/', '?run=r1&branch=b2')).toEqual({ page: 'home', runId: 'r1', branchId: 'b2' });
    expect(hereOf('/', '')).toBeNull();
    expect(hereOf('/suites/s1')).toBeNull();
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
