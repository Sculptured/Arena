// The runs list as table rows: every branch of every run, forks under their parent, with the
// status filter and the text filter the page applies.
import type { Branch, BranchStatus, RunSummary } from '../types';
import { branchTree } from './tree';

export interface BranchRow {
  run: RunSummary;
  branch: Branch;
  /** 0 for a run's first branch; forks are deeper. */
  depth: number;
}

/** Newest run first; each run's branches in tree order. */
export function branchRows(runs: RunSummary[]): BranchRow[] {
  const newest = [...runs].sort((a, b) => b.created_at.localeCompare(a.created_at));
  return newest.flatMap((run) => branchTree(run.branches).map(({ branch, depth }) => ({ run, branch, depth })));
}

export type StatusFilter = 'all' | 'running' | 'passed' | 'failed' | 'other';

export const STATUS_FILTERS: { id: StatusFilter; label: string }[] = [
  { id: 'all', label: 'All' },
  { id: 'running', label: 'Running' },
  { id: 'passed', label: 'Passed' },
  { id: 'failed', label: 'Failed' },
  { id: 'other', label: 'Infra error / interrupted' },
];

export function statusGroup(s: BranchStatus): Exclude<StatusFilter, 'all'> {
  return s === 'running' || s === 'passed' || s === 'failed' ? s : 'other';
}

export function statusCounts(rows: BranchRow[]): Record<StatusFilter, number> {
  const out: Record<StatusFilter, number> = { all: rows.length, running: 0, passed: 0, failed: 0, other: 0 };
  for (const r of rows) out[statusGroup(r.branch.status)] += 1;
  return out;
}

export interface RowFilter {
  status: StatusFilter;
  text: string;
}

/** True when the row is in the status group and every word of the text appears in its name, task, ids or note. */
export function rowMatches(r: BranchRow, f: RowFilter): boolean {
  if (f.status !== 'all' && statusGroup(r.branch.status) !== f.status) return false;
  const hay = [r.run.name ?? '', r.run.task_id, r.run.id, r.branch.id, r.branch.fork_note ?? '', r.branch.status].join(' ').toLowerCase();
  return f.text.toLowerCase().split(/\s+/).filter(Boolean).every((w) => hay.includes(w));
}

/** True while some branch of the run is running (the run can't be deleted then). */
export const runIsRunning = (run: RunSummary) => run.branches.some((b) => b.status === 'running');

/** "Demo: Coordinated team v1 / cvx-2611 / repeat 1": a run's name, else its task. */
export const runTitle = (run: RunSummary) => run.name ?? `task ${run.task_id}`;
