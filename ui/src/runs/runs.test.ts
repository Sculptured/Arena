import { describe, expect, it } from 'vitest';
import type { Branch, RunSummary } from '../types';
import { branchRows, rowMatches, runIsRunning, statusCounts } from './runsTable';

const branch = (id: string, run: string, status: Branch['status'], parent: string | null = null, at = '2026-11-06T10:00:00Z'): Branch =>
  ({ id, run_id: run, parent_id: parent, start_seq: null, team: { id: 't', name: 't', agents: [] }, changes: null, fork_note: parent ? 'model changed' : null, status, error: null, created_at: at, ended_at: null });
const runs: RunSummary[] = [
  { id: 'old', task_id: 'cvx', created_at: '2026-11-05T10:00:00Z', branches: [branch('a1', 'old', 'failed')] },
  { id: 'new', name: 'Team v2', task_id: 'carrier', created_at: '2026-11-06T10:00:00Z',
    branches: [branch('b2', 'new', 'infra_error', 'b1', '2026-11-06T11:00:00Z'), branch('b1', 'new', 'running')] },
];

describe('runs table', () => {
  it('lists newest runs first, each run in branch-tree order', () => {
    expect(branchRows(runs).map((r) => [r.branch.id, r.depth])).toEqual([['b1', 0], ['b2', 1], ['a1', 0]]);
  });
  it('counts and filters by status group and by words', () => {
    const rows = branchRows(runs);
    expect(statusCounts(rows)).toEqual({ all: 3, running: 1, passed: 0, failed: 1, other: 1 });
    expect(rows.filter((r) => rowMatches(r, { status: 'other', text: '' })).map((r) => r.branch.id)).toEqual(['b2']);
    expect(rows.filter((r) => rowMatches(r, { status: 'all', text: 'team MODEL' })).map((r) => r.branch.id)).toEqual(['b2']);
    expect(rows.filter((r) => rowMatches(r, { status: 'all', text: 'cvx' })).map((r) => r.branch.id)).toEqual(['a1']);
  });
  it('a run with a running branch is running', () => {
    expect(runs.map(runIsRunning)).toEqual([false, true]);
  });
});
